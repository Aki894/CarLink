"""R3 process integration: synthetic backends, no Bluetooth/USB hardware touched."""
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import time

MOCK = r'''
import fcntl,json,os,pathlib,signal,socket,sys,time
root=pathlib.Path(sys.argv[1]);role=sys.argv[2];stopping=False
scenario=(root/'scenario').read_text() if (root/'scenario').exists() else ''
def record(event,**fields):
    fd=os.open(root/'events',os.O_WRONLY|os.O_CREAT|os.O_APPEND,0o600)
    os.write(fd,(json.dumps(dict(event=event,role=role,pid=os.getpid(),**fields))+'\n').encode());os.close(fd)
def stop(*_):
    global stopping
    stopping=True
signal.signal(signal.SIGTERM,stop)
lock=None;listener=None
try:
    if role=='carlife' and scenario!='no-live':
        listener=socket.socket(socket.AF_UNIX,socket.SOCK_STREAM);listener.bind(str(root/'live.sock'));listener.listen(1)
    elif role!='carlife':
        lock=open(root/'radio.lock','a')
        try:fcntl.flock(lock,fcntl.LOCK_EX|fcntl.LOCK_NB)
        except BlockingIOError:record('overlap');raise SystemExit(2)
    record('start',args=sys.argv[3:])
    if role=='carplay' and scenario in ('auth','refused','offline','unpaired'):
        reason={'auth':'org.bluez.Error.AuthenticationFailed','refused':'Connection refused (os error 111)','offline':'Host is down','unpaired':'The configured iPhone is no longer paired'}[scenario]
        for _ in range(3 if scenario=='refused' else 1):
            print(json.dumps(dict(event='stage',stage='WaitingForPairedIphone',reason=reason)),flush=True);time.sleep(.05)
    elif role=='carplay':
        args=sys.argv[3:];address=args[args.index('--iphone')+1] if '--iphone' in args else '02:00:00:00:00:01'
        for v in [dict(event='stage',stage='ConnectingBluetooth',address=address),dict(event='stage',stage='WirelessActive'),dict(event='forward_latency',sent=30)]:print(json.dumps(v),flush=True)
    elif role=='hicar':
        if scenario=='pair-hicar' and '--pair' in sys.argv:
            for address in ['not-a-bluetooth-address','02:00:00:00:00:03','02:00:00:00:00:04']:
                print(json.dumps(dict(event='target_selected',address=address)),flush=True)
        print(json.dumps(dict(event='bridge_stats',active=True,connected=scenario!='no-hu',video_frames=20)),flush=True)
    while not stopping:time.sleep(.02)
finally:
    record('stop')
    if listener:listener.close();(root/'live.sock').unlink(missing_ok=True)
    if lock:lock.close()
'''


def check(binary):
    runner = json.loads(os.environ.get('CARLINK_RUST_RUNNER', '[]'))
    with tempfile.TemporaryDirectory(prefix='carlink-manager-') as directory:
        root = Path(directory)
        cp = dict(protocol='carplay', address='02:00:00:00:00:01', name='test iPhone', auto=True, last_used=0)
        hc = dict(protocol='hicar', address='02:00:00:00:00:02', name='test Huawei', auto=True, last_used=0)
        mock = root / 'backend.py'
        mock.write_text(MOCK)
        bt = root / 'bluetoothctl'
        bt.write_text('#!' + sys.executable + '\n' +
            'import json,os,pathlib,sys\nroot=pathlib.Path(' + repr(str(root)) + ')\n' +
            "fd=os.open(root/'events',os.O_WRONLY|os.O_CREAT|os.O_APPEND,0o600)\n" +
            "os.write(fd,(json.dumps(dict(event=('remove' if 'remove' in sys.argv else 'setting'),args=sys.argv[1:]))+" + repr(chr(10)) + ").encode());os.close(fd)\n" +
            "if 'remove' in sys.argv:(root/'scenario').write_text('')\n")
        bt.chmod(0o700)
        config = dict(schema=1, socket=str(root / 'control.sock'), live_socket=str(root / 'live.sock'), state=str(root / 'devices.json'), radio_lock=str(root/'radio-owner.lock'), media_socket=str(root/'media.sock'),
            carlife=[sys.executable, str(mock), str(root), 'carlife'],
            carplay=[sys.executable, str(mock), str(root), 'carplay'], hicar=[sys.executable, str(mock), str(root), 'hicar'],
            devices=[cp, hc], initial='carplay', bluetoothctl=str(bt), connect_seconds=2, stable_seconds=1, pair_seconds=5)
        (root / 'config.json').write_text(json.dumps(config))
        (root / 'config.json').chmod(0o600)
        (root / 'devices.json').write_text(json.dumps(dict(devices=[cp, hc], last=cp, paused=False)))
        (root / 'devices.json').chmod(0o600)
        (root / 'identity-private').write_bytes(b'synthetic persistent identity must remain untouched')
        initial_identity=(root / 'identity-private').read_bytes()
        (root/'scenario').write_text('no-live')
        def ctl(_ok=True, **command):
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
                s.settimeout(12)
                s.connect(str(root / 'control.sock'))
                s.sendall(json.dumps(command).encode() + b'\n')
                data=b''
                while not data.endswith(b'\n'):
                    data += s.recv(32768)
            result=json.loads(data)
            assert bool(result.get('ok'))==_ok, 'unexpected manager control result: '+command['op']
            return result
        def until(predicate, timeout=12):
            deadline=time.monotonic()+timeout
            while time.monotonic()<deadline:
                try:
                    result=ctl(op='status')['status']
                    if predicate(result):return result
                except (OSError, ValueError):pass
                time.sleep(.05)
            raise AssertionError('manager state deadline exceeded')
        def events():
            return [json.loads(l) for l in (root/'events').read_text().splitlines()] if (root/'events').exists() else []
        p=None
        log=(root/'manager.log').open('wb')
        try:
            p=subprocess.Popen([*runner,binary,'manager','run','--config',str(root/'config.json')],stdout=log,stderr=log)
            s=until(lambda s:s['protocol']=='carplay' and s['active'])
            assert not (root/'live.sock').exists(), 'fixture did not exercise pre-USB discovery'
            (root/'scenario').write_text('')
            first_life=s['carlife_pid']
            duplicate=dict(config,socket=str(root/'second/control.sock'),live_socket=str(root/'second/live.sock'),state=str(root/'second/state.json'))
            (root/'duplicate.json').write_text(json.dumps(duplicate));(root/'duplicate.json').chmod(0o600)
            other=subprocess.run([*runner,binary,'manager','run','--config',str(root/'duplicate.json')],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=5)
            assert other.returncode!=0, 'second runtime acquired the same radio'
            (root/'scenario').write_text('no-hu')
            ctl(op='connect',protocol='hicar',address=hc['address'])
            no_hu=until(lambda s:s['protocol']=='hicar' and s['active'])
            time.sleep(2.5)
            assert ctl(op='status')['status']['backend_pid']==no_hu['backend_pid'], 'missing HU stole the wireless source'
            (root/'scenario').write_text('')
            for _ in range(3):
                for protocol,device in [('hicar',hc),('carplay',cp)]:
                    ctl(op='connect',protocol=protocol,address=device['address'])
                    s=until(lambda s:s['protocol']==protocol and s['active'])
                    assert s['carlife_pid']==first_life, 'source switch restarted the USB output'
            ctl(op='connect',protocol='hicar',address=hc['address'])
            s=until(lambda s:s['protocol']=='hicar' and s['active'])
            backend=s['backend_pid']
            # Remembering another source never steals an active phone.
            ctl(op='remember',device=dict(cp,name='renamed test iPhone'))
            assert ctl(op='status')['status']['backend_pid']==backend
            # CarLife can recover independently while the authenticated source remains up.
            os.kill(first_life,signal.SIGKILL)
            s=until(lambda s:s['carlife_pid'] is not None and s['carlife_pid']!=first_life)
            assert s['backend_pid']==backend, 'CarLife recovery killed the source'
            time.sleep(1.2)
            (root/'scenario').write_text('offline')
            ctl(op='connect',protocol='carplay',address=cp['address'])
            s=until(lambda s:s['protocol']=='hicar' and s['active'])
            assert not any(e['event']=='remove' for e in events()), 'ordinary offline erased pairing'
            (root/'scenario').write_text('unpaired')
            ctl(op='connect',protocol='carplay',address=cp['address'])
            until(lambda s:s['protocol']=='carplay' and s['pairing'])
            assert not any(e['event']=='remove' for e in events()), 'missing pairing was unnecessarily removed'
            (root/'scenario').write_text('auth')
            ctl(op='connect',protocol='carplay',address=cp['address'])
            s=until(lambda s:s['protocol']=='carplay' and s['active'] and s['pairing'])
            removed=[e for e in events() if e['event']=='remove']
            assert len(removed)==1 and removed[0]['args']==['--timeout','5','remove',cp['address']]
            until(lambda s:s['active'] and not s['pairing'])
            settings=[e['args'] for e in events() if e['event']=='setting']
            assert ['--timeout','3','discoverable','off'] in settings
            assert ['--timeout','3','pairable','off'] in settings
            ctl(op='disconnect')
            s=ctl(op='status')['status'];assert s['paused'] and s['backend_pid'] is None and s['carlife_pid']
            assert (root/'identity-private').read_bytes()==initial_identity
            ctl(op='shutdown');assert p.wait(timeout=12)==0;p=None
            # Start a fresh lifecycle to verify repeated peer refusal recovery.
            store=json.loads((root/'devices.json').read_text());store['paused']=False;store['last']=cp
            (root/'devices.json').write_text(json.dumps(store))
            (root/'scenario').write_text('refused')
            p=subprocess.Popen([*runner,binary,'manager','run','--config',str(root/'config.json')],stdout=log,stderr=log)
            until(lambda s:s['protocol']=='carplay' and s['active'])
            removed=[e for e in events() if e['event']=='remove'];assert len(removed)==2
            assert all(e['args'][-1]==cp['address'] for e in removed)
            assert not any(e['event']=='overlap' for e in events()), 'wireless backend overlap'
            # Successful-use/favorite metadata contains historical copies. A
            # later record edit disabling auto must take effect for both policies.
            stable=until(lambda s:s['active'] and not s['pairing'])
            ctl(op='policy',mode='favorite',protocol='carplay',address=cp['address'])
            ctl(op='remember',device=dict(cp,auto=False))
            ctl(op='remember',device=dict(hc,auto=False))
            assert ctl(op='status')['status']['backend_pid']==stable['backend_pid'], 'record edit stole the active session'
            ctl(op='disconnect')
            for mode in ['favorite','last']:
                if mode=='last':ctl(op='policy',mode='last')
                marker=len(events())
                ctl(op='resume')
                time.sleep(2.3)
                assert ctl(op='status')['status']['backend_pid'] is None, f'{mode} reused stale auto=true metadata'
                assert not any(e['event']=='start' and e['role']!='carlife' for e in events()[marker:]), 'disabled device was briefly reconnected'
                ctl(op='pause')
            ctl(op='remember',device=cp);ctl(op='remember',device=hc)
            ctl(op='resume')
            stable=until(lambda s:s['active'] and not s['pairing'])
            # Manual policy preserves the current session, then waits for a user
            # after disconnect/resume and across a manager restart.
            ctl(op='policy',mode='manual')
            assert ctl(op='status')['status']['backend_pid']==stable['backend_pid']
            ctl(op='disconnect');ctl(op='resume')
            time.sleep(2.3)
            assert ctl(op='status')['status']['backend_pid'] is None, 'manual policy automatically selected a phone'
            (root/'scenario').write_text('')
            ctl(op='connect',protocol='hicar',address=hc['address'])
            until(lambda s:s['protocol']=='hicar' and s['active'])
            ctl(op='shutdown');assert p.wait(timeout=12)==0;p=None
            p=subprocess.Popen([*runner,binary,'manager','run','--config',str(root/'config.json')],stdout=log,stderr=log)
            until(lambda s:s['carlife_pid'] is not None)
            time.sleep(2.3)
            assert ctl(op='status')['status']['backend_pid'] is None, 'manual policy was lost at restart'
            # A missing favorite never falls back to another registered phone.
            (root/'scenario').write_text('offline')
            ctl(op='policy',mode='favorite',protocol='carplay',address=cp['address'])
            marker=len(events())
            ctl(op='resume')
            until(lambda s:s['protocol']=='carplay' and s['backend_pid'] is not None)
            time.sleep(5)
            attempts=[e for e in events()[marker:] if e['event']=='start' and e['role']!='carlife']
            assert attempts and all(e['role']=='carplay' for e in attempts), 'favorite policy fell back to another phone'
            (root/'scenario').write_text('')
            ctl(op='connect',protocol='hicar',address=hc['address'])
            chosen=until(lambda s:s['protocol']=='hicar' and s['active'])
            time.sleep(2.3)
            assert ctl(op='status')['status']['backend_pid']==chosen['backend_pid'], 'favorite stole the explicitly selected session'
            # Unknown HiCar pairing creates a pending record only after a valid
            # selected transport; stable delivered media is the persistence gate.
            (root/'scenario').write_text('pair-hicar')
            ctl(op='pair',protocol='hicar')
            selected=until(lambda s:s['protocol']=='hicar' and s['active'] and s['target'] and s['target']['address']=='02:00:00:00:00:03')
            assert selected['pairing']
            assert not any(d['address']=='02:00:00:00:00:03' for d in ctl(op='devices')['devices']), 'pending target persisted before stable media'
            paired=until(lambda s:s['active'] and not s['pairing'])
            assert paired['target']['address']=='02:00:00:00:00:03', 'later candidate stole the pairing target'
            store=json.loads((root/'devices.json').read_text())
            assert store['last']['address']=='02:00:00:00:00:03'
            assert len(store['devices'])==3 and any(d['protocol']=='hicar' and d['address']=='02:00:00:00:00:03' for d in store['devices'])
            latest=next(e for e in reversed(events()) if e['event']=='start' and e['role']=='hicar')
            assert '--pair' in latest['args'] and '--peer' not in latest['args']
            before=len([e for e in events() if e['event']=='remove'])
            ctl(_ok=False,op='forget',protocol='hicar',address='02:00:00:00:00:03')
            assert len([e for e in events() if e['event']=='remove'])==before
            ctl(op='forget',protocol='hicar',address='02:00:00:00:00:03',confirm=True)
            removed=[e for e in events() if e['event']=='remove']
            assert len(removed)==before+1 and removed[-1]['args']==['--timeout','5','remove','02:00:00:00:00:03']
            remaining=ctl(op='devices')['devices']
            assert {d['address'] for d in remaining}=={cp['address'],hc['address']}
            # The initial config is a first-run seed, never a way to resurrect a
            # device explicitly forgotten in the persistent store.
            ctl(op='forget',protocol='carplay',address=cp['address'],confirm=True)
            ctl(op='shutdown');assert p.wait(timeout=12)==0;p=None
            p=subprocess.Popen([*runner,binary,'manager','run','--config',str(root/'config.json')],stdout=log,stderr=log)
            until(lambda s:s['carlife_pid'] is not None)
            remaining=ctl(op='devices')['devices']
            assert len(remaining)==1 and remaining[0]['address']==hc['address'], 'forgotten config seed reappeared'
            assert (root/'identity-private').read_bytes()==initial_identity
            ctl(op='shutdown');assert p.wait(timeout=12)==0;p=None
            starts=[e for e in events() if e['event']=='start' and e['role']=='carplay']
            assert any('--pair' in e['args'] and '--iphone' in e['args'] for e in starts)
            print(json.dumps(dict(connection_manager='passed',switches=7,usb_pid_preserved=True,
                independent_usb_recovery=True,radio_overlap=False,offline_pairing_retained=True,
                authentication_and_repeated_refusal_repaired=True,target_only=True,identity_unchanged=True,
                manual_and_favorite_policy=True,current_auto_record_respected=True,hicar_enrollment_stability_gate=True,forget_confirmation_and_persistence=True)))
        except BaseException:
            log.flush()
            print((root/'manager.log').read_text(encoding='utf-8',errors='replace')[-8000:],file=sys.stderr)
            print(json.dumps(events()[-24:]),file=sys.stderr)
            raise
        finally:
            if p and p.poll() is None:
                p.terminate()
                try:p.wait(timeout=12)
                except subprocess.TimeoutExpired:p.kill();p.wait(timeout=5)
            log.close()


if __name__=='__main__':
    check(str(Path(sys.argv[1]).resolve()))
