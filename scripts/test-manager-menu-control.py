"""Real manager + native menu/mux + simulated HU/source integration.

Usage: test-manager-menu-control.py BINARY NATIVE_LIBRARY FONT
Run same-architecture artifacts on Linux CI or H3. No real radio/USB is used.
"""
import collections
import ctypes
import hashlib
import json
import os
from pathlib import Path
import re
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time


MOCK = r'''
import fcntl,json,os,pathlib,signal,socket,struct,sys,time
root=pathlib.Path(sys.argv[1]);role=sys.argv[2];stopping=False

def record(event,**fields):
    with (root/'events.jsonl').open('a') as output:
        output.write(json.dumps(dict(event=event,role=role,pid=os.getpid(),**fields))+'\n')

def stop(*_):
    global stopping
    stopping=True

def wire(kind,flags,payload=b'',pts=0):
    return struct.pack('<IBBHQ',12+len(payload),kind,flags,0,pts)+payload

signal.signal(signal.SIGTERM,stop)
record('start')
stream=None;lock=None
try:
    if role=='carlife':
        # The test owns the HU socket, bound only after the manager is ready.
        print(json.dumps(dict(event='carlife_state',detail='AwaitingVideoSource')),flush=True)
        while not stopping:time.sleep(.02)
    else:
        lock=(root/'backend-radio.lock').open('a')
        try:fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        except BlockingIOError:
            record('overlap');raise SystemExit(2)
        args=sys.argv[3:]
        path=os.environ.get('CARLINK_LIVE_SOCKET')
        if '--live-socket' in args:path=args[args.index('--live-socket')+1]
        assert path and pathlib.Path(path)==root/'source.sock'
        deadline=time.monotonic()+12
        while not stopping:
            try:
                stream=socket.socket(socket.AF_UNIX);stream.settimeout(.5);stream.connect(path);break
            except OSError:
                stream.close();stream=None
                if time.monotonic()>deadline:raise RuntimeError('source socket deadline')
                time.sleep(.03)
        if stream is None:raise SystemExit(0)
        fixture=(root/(role+'.h264')).read_bytes()
        pcm=struct.pack('<hh',600 if role=='carplay' else 1200,-600 if role=='carplay' else -1200)*882
        stream.sendall(wire(1,0,struct.pack('<III',800,480,30))+wire(4,1))
        if role=='carplay':
            for event in [dict(event='stage',stage='WirelessActive'),dict(event='forward_latency',sent=1)]:
                print(json.dumps(event),flush=True)
        else:
            print(json.dumps(dict(event='bridge_stats',active=True,connected=True,video_frames=1)),flush=True)
        record('connected')
        stream.setblocking(False)
        read_buffer=b'';count=0;next_at=time.monotonic()
        while not stopping:
            now=time.monotonic()
            if now>=next_at:
                stream.settimeout(.5)
                pts=time.monotonic_ns()//1000
                stream.sendall(wire(3,0,pcm,pts))
                if count%5==0:stream.sendall(wire(2,1,fixture,pts))
                stream.setblocking(False)
                count+=1;next_at=now+.02
            try:
                chunk=stream.recv(16384)
                if not chunk:break
                read_buffer+=chunk
                if len(read_buffer)>1024*1024:raise RuntimeError('feedback receive limit')
            except BlockingIOError:pass
            while len(read_buffer)>=4:
                size=struct.unpack_from('<I',read_buffer)[0]
                if not 12<=size<=16384:raise RuntimeError('feedback frame bound')
                if len(read_buffer)<4+size:break
                body=read_buffer[4:4+size];read_buffer=read_buffer[4+size:]
                if body[0]!=129:raise RuntimeError('feedback kind')
                message=json.loads(body[12:])
                if message.get('type') in ('key-press','touch'):
                    record('input',type=message['type'],code=message.get('code'))
            time.sleep(.002)
finally:
    if stream:stream.close()
    if lock:lock.close()
    record('stop')
'''


def wire(kind, flags=0, payload=b'', pts=0):
    return struct.pack('<IBBHQ',12+len(payload),kind,flags,0,pts)+payload


def nal_types(data):
    return {data[match.end()]&31 for match in re.finditer(b'\x00\x00(?:\x00)?\x01',data)
            if match.end()<len(data)}


def render_sources(library, font):
    """Generate one independent real SPS/PPS/IDR per synthetic source."""
    native=ctypes.CDLL(str(library))
    point=ctypes.POINTER(ctypes.c_uint32)
    native.carlink_menu_create.argtypes=[ctypes.c_char_p]
    native.carlink_menu_create.restype=ctypes.c_void_p
    native.carlink_menu_destroy.argtypes=[ctypes.c_void_p]
    native.carlink_menu_destroy.restype=None
    native.carlink_menu_render.argtypes=[ctypes.c_void_p,ctypes.POINTER(point),point,
        ctypes.c_uint32,ctypes.c_uint32,ctypes.POINTER(ctypes.c_ubyte),ctypes.c_uint32]
    native.carlink_menu_render.restype=ctypes.c_int
    context=native.carlink_menu_create(os.fsencode(font))
    assert context, 'native source fixture renderer did not initialize'
    try:
        frames={}
        for role in ('carplay','hicar'):
            label='Synthetic '+role+' source'
            points=(ctypes.c_uint32*len(label))(*(ord(c) for c in label))
            rows=(point*1)(ctypes.cast(points,point));lengths=(ctypes.c_uint32*1)(len(label))
            output=(ctypes.c_ubyte*(512*1024))()
            size=native.carlink_menu_render(context,rows,lengths,1,0,output,len(output))
            assert 200<size<=len(output), 'native source fixture rendering failed'
            frame=bytes(output[:size]);assert {5,7,8}<=nal_types(frame)
            frames[role]=frame
        assert frames['carplay']!=frames['hicar']
        return frames
    finally:
        native.carlink_menu_destroy(context)


class Headunit:
    """Drain continuously, retaining bounded metadata rather than media copies."""
    def __init__(self, stream):
        self.stream=stream;stream.settimeout(.2)
        self.stop=threading.Event();self.lock=threading.Lock()
        self.frames=collections.deque(maxlen=256);self.sequence=0;self.error=None
        self.reader=threading.Thread(target=self.read,daemon=True);self.reader.start()

    def read(self):
        data=b''
        try:
            while not self.stop.is_set():
                try:chunk=self.stream.recv(16384)
                except socket.timeout:continue
                if not chunk:
                    if not self.stop.is_set():raise AssertionError('mux disconnected the HU')
                    break
                data+=chunk
                assert len(data)<=1024*1024+4, 'HU receive buffer exceeded'
                while len(data)>=4:
                    size=struct.unpack_from('<I',data)[0]
                    assert 12<=size<=1024*1024, 'invalid HU frame length'
                    if len(data)<size+4:break
                    body=data[4:size+4];data=data[size+4:]
                    kind,flags,reserved,pts=struct.unpack_from('<BBHQ',body)
                    payload=body[12:]
                    assert reserved==0 and kind in (1,2,3,4,5,6)
                    if kind==1:assert payload==struct.pack('<III',800,480,30)
                    if kind==2:assert flags&1 and {5,7,8}<=nal_types(payload)
                    if kind==3:assert payload and len(payload)%4==0
                    with self.lock:
                        self.sequence+=1
                        self.frames.append(dict(sequence=self.sequence,kind=kind,flags=flags,
                            size=len(payload),sha256=hashlib.sha256(payload).hexdigest(),pts=pts,arrival=time.monotonic()))
        except BaseException as error:
            if not self.stop.is_set():self.error=error

    def snapshot(self, after=0):
        if self.error:raise AssertionError('HU framing/connection failed') from self.error
        with self.lock:return [frame.copy() for frame in self.frames if frame['sequence']>after]

    def mark(self):
        with self.lock:return self.sequence

    def feedback(self, value):
        self.stream.sendall(wire(129,payload=json.dumps(value).encode()))

    def close(self):
        self.stop.set()
        try:self.stream.shutdown(socket.SHUT_RDWR)
        except OSError:pass
        self.stream.close();self.reader.join(timeout=2)
        assert not self.reader.is_alive(), 'HU reader did not terminate'


def check(binary, library, font):
    runner=json.loads(os.environ.get('CARLINK_RUST_RUNNER','[]'))
    fixtures=render_sources(library,font)
    source_hash={role:hashlib.sha256(frame).hexdigest() for role,frame in fixtures.items()}
    pcm_hash={role:hashlib.sha256(struct.pack('<hh',level,-level)*882).hexdigest()
        for role,level in [('carplay',600),('hicar',1200)]}
    with tempfile.TemporaryDirectory(prefix='carlink-menu-control-') as directory:
        root=Path(directory)
        for role,frame in fixtures.items():(root/(role+'.h264')).write_bytes(frame)
        backend=root/'backend.py';backend.write_text(MOCK,encoding='utf-8')
        cp=dict(protocol='carplay',address='02:00:00:00:00:01',name='Synthetic iPhone',auto=True,last_used=0)
        hc=dict(protocol='hicar',address='02:00:00:00:00:02',name='Synthetic Huawei',auto=True,last_used=0)
        state=root/'devices.json'
        state.write_text(json.dumps(dict(devices=[cp,hc],policy='manual',paused=False)),encoding='utf-8');state.chmod(0o600)
        config=dict(schema=1,socket=str(root/'control.sock'),live_socket=str(root/'live.sock'),
            source_socket=str(root/'source.sock'),menu_library=str(library),menu_font=str(font),
            state=str(state),radio_lock=str(root/'manager-radio.lock'),media_socket=str(root/'media.sock'),
            carlife=[sys.executable,str(backend),str(root),'carlife'],
            carplay=[sys.executable,str(backend),str(root),'carplay'],
            hicar=[sys.executable,str(backend),str(root),'hicar','--live-socket',str(root/'live.sock')],
            devices=[cp,hc],connect_seconds=10,stable_seconds=2,pair_seconds=5)
        config_path=root/'config.json';config_path.write_text(json.dumps(config),encoding='utf-8');config_path.chmod(0o600)
        process=None;hu=None;listener=None
        output=(root/'manager.log').open('wb')
        def ctl(**command):
            with socket.socket(socket.AF_UNIX) as stream:
                stream.settimeout(12);stream.connect(str(root/'control.sock'))
                stream.sendall(json.dumps(command).encode()+b'\n');data=b''
                while not data.endswith(b'\n'):
                    chunk=stream.recv(32768)
                    assert chunk, 'manager control closed without response'
                    data+=chunk;assert len(data)<=32768, 'control response exceeded bound'
            result=json.loads(data)
            assert result.get('ok'), 'manager operation failed: '+command['op']
            return result
        def until(predicate, label, timeout=15):
            deadline=time.monotonic()+timeout
            while time.monotonic()<deadline:
                if hu:hu.snapshot()
                if process.poll() is not None:raise AssertionError('manager exited while waiting for '+label)
                result=predicate()
                if result:return result
                time.sleep(.025)
            raise AssertionError('deadline waiting for '+label)
        def status_where(predicate):
            try:status=ctl(op='status')['status']
            except (OSError,ValueError):return None
            return status if predicate(status) else None
        def frame_where(predicate, after):
            return next((frame for frame in hu.snapshot(after) if predicate(frame)),None)
        def events():
            path=root/'events.jsonl'
            return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []
        try:
            process=subprocess.Popen([*runner,str(binary),'manager','run','--config',str(config_path)],stdout=output,stderr=output)
            ready=until(lambda:status_where(lambda s:s['carlife_pid'] is not None and s['menu_visible']), 'initial menu state')
            first_life=ready['carlife_pid']
            assert ready['backend_pid'] is None and ready['protocol'] is None, 'manual policy started a source'
            assert not (root/'live.sock').exists(), 'fixture bound the HU before manager startup'
            listener=socket.socket(socket.AF_UNIX);listener.bind(str(root/'live.sock'));listener.listen(1);listener.settimeout(12)
            connection,_=listener.accept();hu=Headunit(connection)
            initial=until(lambda:frame_where(lambda f:f['kind']==2,0), 'initial native menu IDR')
            # Lexus-style focus movement and confirmation pass through the real
            # mux, which supplies its own menu_rows snapshot to the manager.
            marker=hu.mark();hu.feedback(dict(type='key-press',code=6))
            until(lambda:frame_where(lambda f:f['kind']==2 and f['sha256']!=initial['sha256'],marker), 'focused second menu row')
            hu.feedback(dict(type='key-press',code=2))
            selected=until(lambda:status_where(lambda s:s['protocol']=='hicar' and s['active'] and not s['menu_visible']), 'HiCar selected by focus/enter')
            assert selected['target']['address']==hc['address'] and selected['carlife_pid']==first_life
            until(lambda:frame_where(lambda f:f['kind']==2 and f['sha256']==source_hash['hicar'],marker), 'HiCar source video')
            until(lambda:frame_where(lambda f:f['kind']==3 and f['sha256']==pcm_hash['hicar'],marker), 'HiCar PCM forwarding')
            old_backend=selected['backend_pid']
            marker=hu.mark();ctl(op='menu',visible=True)
            until(lambda:status_where(lambda s:s['menu_visible'] and s['backend_pid']==old_backend), 'overlay without source restart')
            menu=until(lambda:frame_where(lambda f:f['kind']==2 and f['sha256'] not in source_hash.values(),marker), 'native menu replacing only source video')
            until(lambda:len([f for f in hu.snapshot(menu['sequence']) if f['kind']==3 and f['sha256']==pcm_hash['hicar']])>=5, 'continuous PCM while menu is visible')
            # Cover a full static-menu heartbeat, not just the first redraw.
            time.sleep(1.2)
            audio=[f['arrival'] for f in hu.snapshot(max(0,marker-6)) if f['kind']==3 and f['sha256']==pcm_hash['hicar']]
            assert len(audio)>=20
            menu_pcm_max_gap_ms=max((b-a)*1000 for a,b in zip(audio,audio[1:]))
            assert menu_pcm_max_gap_ms<250, 'menu painting blocked sustained PCM forwarding'

            assert not any(f['kind']==2 and f['sha256']==source_hash['hicar'] for f in hu.snapshot(menu['sequence'])), 'source video leaked through the menu'
            assert ctl(op='status')['status']['backend_pid']==old_backend, 'opening the menu restarted the source'
            # Hit row zero with an actual down/up pair. No private menu_rows or
            # direct connect operation is provided by this simulated HU.
            marker=hu.mark()
            for down in (True,False):
                hu.feedback(dict(type='touch',contacts=[dict(id=0,x=.5,y=.2,down=down)]))
                time.sleep(.06)
            selected=until(lambda:status_where(lambda s:s['protocol']=='carplay' and s['active'] and not s['menu_visible']), 'CarPlay selected by menu touch')
            assert selected['target']['address']==cp['address'] and selected['carlife_pid']==first_life
            assert selected['backend_pid']!=old_backend
            until(lambda:frame_where(lambda f:f['kind']==2 and f['sha256']==source_hash['carplay'],marker), 'CarPlay source video after menu exit')
            until(lambda:frame_where(lambda f:f['kind']==3 and f['sha256']==pcm_hash['carplay'],marker), 'CarPlay PCM after source switch')
            assert not any(e['event']=='input' and e['role']=='hicar' for e in events()), 'menu input was forwarded into HiCar'
            hu.feedback(dict(type='key-press',code=1))
            until(lambda:any(e['event']=='input' and e['role']=='carplay' and e['code']==1 for e in events()), 'projection input restored after menu exit')
            records=events()
            assert len([e for e in records if e['event']=='start' and e['role']=='carlife'])==1
            assert [e['role'] for e in records if e['event']=='connected']==['hicar','carplay']
            assert not any(e['event']=='overlap' for e in records), 'wireless source overlap'
            hu.snapshot();hu.close();hu=None
            ctl(op='shutdown');assert process.wait(timeout=12)==0;process=None
            assert not os.path.lexists(root/'control.sock') and not os.path.lexists(root/'source.sock'), 'manager left runtime sockets'
            print(json.dumps(dict(manager_menu_control='passed',native_idr=True,focus_enter_hicar=True,
                touch_select_carplay=True,menu_audio_preserved=True,menu_pcm_max_gap_ms=round(menu_pcm_max_gap_ms,1),source_video_restored=True,
                projection_input_restored=True,carlife_pid_preserved=True,source_overlap=False)))
        except BaseException:
            output.flush()
            print((root/'manager.log').read_text(encoding='utf-8',errors='replace')[-12000:],file=sys.stderr)
            print(json.dumps(events()[-24:]),file=sys.stderr)
            if hu:
                try:print(json.dumps(hu.snapshot()[-16:]),file=sys.stderr)
                except BaseException as error:print(str(error),file=sys.stderr)
            raise
        finally:
            if hu:hu.close()
            if listener:listener.close()
            if process and process.poll() is None:
                process.terminate()
                try:process.wait(timeout=12)
                except subprocess.TimeoutExpired:process.kill();process.wait(timeout=5)
            output.close()


if __name__=='__main__':
    if len(sys.argv)!=4:raise SystemExit('Usage: test-manager-menu-control.py BINARY NATIVE_LIBRARY FONT')
    check(*(Path(value).resolve() for value in sys.argv[1:]))
