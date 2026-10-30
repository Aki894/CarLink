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
            for address in ['not-a-bluetooth-address','02:00:00:00:00:03','02:00:00:00:00:02','02:00:00:00:00:04']:
                print(json.dumps(dict(event='target_selected',address=address)),flush=True)
        args=sys.argv[3:];peer=args[args.index('--peer')+1] if '--peer' in args else None
        if not (scenario=='hicar-first-offline' and peer=='02:00:00:00:00:02'):
            print(json.dumps(dict(event='bridge_stats',active=True,connected=scenario!='no-hu',video_frames=20)),flush=True)
    usb_ready=False
    while not stopping:
        if role=='carlife':
            current=(root/'scenario').read_text() if (root/'scenario').exists() else ''
            ready=current not in ('no-live','no-hu')
            if ready!=usb_ready:
                event=(dict(event='carlife_state',detail='AwaitingVideoSource') if ready
                       else dict(event='session_error',detail='fixture output disconnected'))
                print(json.dumps(event),flush=True)
                usb_ready=ready
        time.sleep(.02)
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
        hicar_identity = root/'hicar-identity.json'
        hicar_identity.write_text(json.dumps(dict(schema=1, device_id='ab'*32,
            pin='012345', name='CarLink')))
        hicar_identity.chmod(0o600)
        bt = root / 'bluetoothctl'
        bt.write_text('#!' + sys.executable + '\n' +
            'import json,os,pathlib,sys\nroot=pathlib.Path(' + repr(str(root)) + ')\n' +
            "fd=os.open(root/'events',os.O_WRONLY|os.O_CREAT|os.O_APPEND,0o600)\n" +
            "operation='remove' if 'remove' in sys.argv else ('disconnect' if 'disconnect' in sys.argv else 'setting')\n" +
            "failed=operation=='disconnect' and (root/'scenario').exists() and (root/'scenario').read_text()=='disconnect-fail'\n" +
            "os.write(fd,(json.dumps(dict(event=operation,args=sys.argv[1:],failed=failed))+" + repr(chr(10)) + ").encode());os.close(fd)\n" +
            "if failed:raise SystemExit(1)\n" +
            "if 'remove' in sys.argv:(root/'scenario').write_text('')\n")
        bt.chmod(0o700)
        config = dict(schema=1, socket=str(root / 'control.sock'), live_socket=str(root / 'live.sock'), state=str(root / 'devices.json'), radio_lock=str(root/'radio-owner.lock'), media_socket=str(root/'media.sock'),
            carlife=[sys.executable, str(mock), str(root), 'carlife'],
            carplay=[sys.executable, str(mock), str(root), 'carplay'], hicar=[sys.executable, str(mock), str(root), 'hicar', '--identity', str(hicar_identity)],
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
            # WirelessActive and forward.sent can describe data accepted by a
            # mux before the HU exists. They must not commit successful use yet.
            first_life=s['carlife_pid'];first_backend=s['backend_pid']
            time.sleep(1.3)
            before_usb=ctl(op='status')['status']
            assert before_usb['active'] and not before_usb['usb_connected']
            assert before_usb['backend_pid']==first_backend and before_usb['target']['last_used']==0
            store=json.loads((root/'devices.json').read_text())
            assert all(d['last_used']==0 for d in store['devices']) and store['last']['last_used']==0, 'pre-USB source updated successful-use history'
            log.flush()
            assert 'manager_stable_session' not in (root/'manager.log').read_text(), 'pre-USB source committed a stable session'
            (root/'scenario').write_text('')
            after_usb=until(lambda s:s['usb_connected'] and s['active'] and s['target']['last_used']>0)
            assert after_usb['carlife_pid']==first_life and after_usb['backend_pid']==first_backend, 'HU arrival restarted an existing process'
            log.flush()
            stable_before_hu_loss=(root/'manager.log').read_text().count('manager_stable_session')
            assert stable_before_hu_loss==1, 'HU arrival did not commit exactly one stable session'
            duplicate=dict(config,socket=str(root/'second/control.sock'),live_socket=str(root/'second/live.sock'),state=str(root/'second/state.json'))
            (root/'duplicate.json').write_text(json.dumps(duplicate));(root/'duplicate.json').chmod(0o600)
            other=subprocess.run([*runner,binary,'manager','run','--config',str(root/'duplicate.json')],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=5)
            assert other.returncode!=0, 'second runtime acquired the same radio'
            (root/'scenario').write_text('no-hu')
            until(lambda s:not s['usb_connected'])
            marker=len(events())
            ctl(op='connect',protocol='hicar',address=hc['address'])
            no_hu=until(lambda s:s['protocol']=='hicar' and s['active'])
            cleanup=[e for e in events()[marker:] if e['event']=='disconnect']
            assert len(cleanup)==1 and cleanup[0]['args']==['--timeout','3','disconnect',cp['address']], 'CarPlay-to-HiCar did not disconnect only its old target'
            assert not any(e['event']=='remove' for e in events()[marker:]), 'source switch removed a pairing bond'
            time.sleep(2.5)
            no_hu_now=ctl(op='status')['status']
            assert no_hu_now['backend_pid']==no_hu['backend_pid'], 'missing HU stole the wireless source'
            assert not no_hu_now['usb_connected'] and no_hu_now['target']['last_used']==0
            log.flush()
            assert (root/'manager.log').read_text().count('manager_stable_session')==stable_before_hu_loss, 'missing HU committed a new successful source'
            (root/'scenario').write_text('')
            until(lambda s:s['usb_connected'])
            for _ in range(3):
                for protocol,device in [('hicar',hc),('carplay',cp)]:
                    old=ctl(op='status')['status'];marker=len(events())
                    ctl(op='connect',protocol=protocol,address=device['address'])
                    s=until(lambda s:s['protocol']==protocol and s['active'])
                    assert s['carlife_pid']==first_life, 'source switch restarted the USB output'
                    cleanup=[e for e in events()[marker:] if e['event']=='disconnect']
                    if old['protocol']==protocol and old['target']['address']==device['address']:
                        assert not cleanup and s['backend_pid']==old['backend_pid'] and s['generation']==old['generation'], 'duplicate selection restarted its receiver'
                    else:
                        assert len(cleanup)==1 and cleanup[0]['args']==['--timeout','3','disconnect',old['target']['address']], 'switch cleaned the wrong Bluetooth target'
                    assert not any(e['event']=='remove' for e in events()[marker:]), 'ordinary source switching removed a bond'
            # Bluetooth may already be disconnected or the controller may reject
            # cleanup. Both directions must still transfer ownership successfully.
            (root/'scenario').write_text('disconnect-fail')
            for protocol,device in [('hicar',hc),('carplay',cp)]:
                old=ctl(op='status')['status'];marker=len(events())
                ctl(op='connect',protocol=protocol,address=device['address'])
                switched=until(lambda s:s['protocol']==protocol and s['active'])
                assert switched['carlife_pid']==first_life
                cleanup=[e for e in events()[marker:] if e['event']=='disconnect']
                assert len(cleanup)==1 and cleanup[0]['failed'] and cleanup[0]['args']==['--timeout','3','disconnect',old['target']['address']], 'disconnect failure fixture was not exercised'
                assert not any(e['event']=='remove' for e in events()[marker:]), 'cleanup failure removed a pairing bond'
            (root/'scenario').write_text('')
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
            time.sleep(4.5)
            s=ctl(op='status')['status']
            assert s['protocol']=='carplay' and not s['active'], 'offline target switched to another protocol'
            assert json.loads((root/'devices.json').read_text())['selected_protocol']=='carplay'
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
            # Re-enrolling an existing transport must not replace the user's
            # chosen alias or explicit auto-connect preference with defaults.
            preferred_cp=dict(cp,name='Daily iPhone - user alias',auto=False)
            ctl(op='remember',device=preferred_cp)
            ctl(op='pair',protocol='carplay')
            until(lambda s:s['protocol']=='carplay' and s['active'] and s['pairing'])
            stable=until(lambda s:s['active'] and not s['pairing'])
            enrolled=next(d for d in ctl(op='devices')['devices'] if d['protocol']=='carplay' and d['address']==cp['address'])
            assert enrolled['name']==preferred_cp['name'] and enrolled['auto'] is False, 're-pairing reset user device preferences'
            ctl(op='remember',device=cp)
            # Manual policy preserves the current session, then waits for a user
            # after disconnect/resume and across a manager restart.
            ctl(op='policy',mode='manual')
            assert ctl(op='status')['status']['backend_pid']==stable['backend_pid']
            ctl(op='disconnect')
            cleanup_count=len([e for e in events() if e['event']=='disconnect'])
            ctl(op='pause');ctl(op='resume');ctl(op='status')
            time.sleep(2.3)
            assert ctl(op='status')['status']['backend_pid'] is None, 'manual policy automatically selected a phone'
            assert len([e for e in events() if e['event']=='disconnect'])==cleanup_count, 'idle pause/resume/status repeated Bluetooth cleanup'
            (root/'scenario').write_text('')
            ctl(op='connect',protocol='hicar',address=hc['address'])
            until(lambda s:s['protocol']=='hicar' and s['active'])
            ctl(op='shutdown');assert p.wait(timeout=12)==0;p=None
            p=subprocess.Popen([*runner,binary,'manager','run','--config',str(root/'config.json')],stdout=log,stderr=log)
            until(lambda s:s['carlife_pid'] is not None)
            time.sleep(2.3)
            assert ctl(op='status')['status']['backend_pid'] is None, 'manual policy was lost at restart'
            # A missing favorite never falls back to another registered phone.
            cp_other=dict(cp,address='02:00:00:00:00:05',name='other iPhone',last_used=9999999999)
            ctl(op='remember',device=cp_other)
            (root/'scenario').write_text('offline')
            ctl(op='pause')
            ctl(op='policy',mode='favorite',protocol='carplay',address=cp['address'])
            marker=len(events())
            ctl(op='resume')
            until(lambda s:s['protocol']=='carplay' and s['backend_pid'] is not None)
            time.sleep(5)
            attempts=[e for e in events()[marker:] if e['event']=='start' and e['role']!='carlife']
            assert attempts and all(e['role']=='carplay' and e['args'][e['args'].index('--iphone')+1]==cp['address'] for e in attempts), 'favorite policy ignored the favorite address or fell back'
            ctl(op='forget',protocol='carplay',address=cp_other['address'],confirm=True)
            (root/'scenario').write_text('')
            ctl(op='connect',protocol='hicar',address=hc['address'])
            chosen=until(lambda s:s['protocol']=='hicar' and s['active'])
            time.sleep(2.3)
            assert ctl(op='status')['status']['backend_pid']==chosen['backend_pid'], 'favorite stole the explicitly selected session'
            ctl(op='policy',mode='favorite',protocol='carplay',address=cp['address'])
            current=ctl(op='status')['status']
            assert current['protocol']=='hicar' and current['backend_pid']==chosen['backend_pid'], 'future preference relabeled or replaced the active HiCar session'
            ctl(op='connect',protocol='hicar',address=hc['address'])
            assert ctl(op='status')['status']['backend_pid']==chosen['backend_pid'], 'reselecting the active target restarted it'
            selected_store=json.loads((root/'devices.json').read_text())
            assert selected_store['policy']=='last' and selected_store['selected_protocol']=='hicar', 'manual target did not override the future favorite'
            # Unknown HiCar pairing creates a pending record only after a valid
            # selected transport; stable delivered media is the persistence gate.
            (root/'scenario').write_text('pair-hicar')
            # A connection window accepts new or registered phones. Here a new
            # valid phone arrives first; later known/new candidates cannot steal it.
            ctl(op='remember',device=dict(hc,auto=False))
            ctl(op='policy',mode='favorite',protocol='carplay',address=cp['address'])
            ctl(op='pair',protocol='hicar')
            assert json.loads((root/'devices.json').read_text())['policy']=='last', 'cross-protocol pairing left an incompatible favorite'
            selected=until(lambda s:s['protocol']=='hicar' and s['active'] and s['target'] and s['target']['address']=='02:00:00:00:00:03')
            assert selected['pairing']
            assert not any(d['address']=='02:00:00:00:00:03' for d in ctl(op='devices')['devices']), 'pending target persisted before stable media'
            pending_hc=dict(protocol='hicar',address='02:00:00:00:00:03',name='Huawei - edited while connecting',auto=False,last_used=0)
            ctl(op='remember',device=pending_hc)
            paired=until(lambda s:s['active'] and not s['pairing'])
            assert paired['target']['address']=='02:00:00:00:00:03', 'later candidate stole the pairing target'
            store=json.loads((root/'devices.json').read_text())
            assert store['last']['address']=='02:00:00:00:00:03'
            assert len(store['devices'])==3 and any(d['protocol']=='hicar' and d['address']=='02:00:00:00:00:03' for d in store['devices'])
            edited=next(d for d in store['devices'] if d['address']==pending_hc['address'])
            assert edited['name']==pending_hc['name'] and edited['auto'] is False, 'stable-session commit overwrote an in-flight user edit'
            assert store['last']['name']==pending_hc['name'] and store['last']['auto'] is False
            latest=next(e for e in reversed(events()) if e['event']=='start' and e['role']=='hicar')
            assert '--pair' in latest['args'] and '--peer' not in latest['args']
            excluded=[latest['args'][i+1] for i,value in enumerate(latest['args'][:-1]) if value=='--exclude-peer']
            assert not excluded, 'connection window excluded a registered phone instead of allowing new or existing devices'
            known=[e for e in events() if e['event']=='start' and e['role']=='hicar' and '--peer' in e['args']]
            assert known and all('--exclude-peer' not in e['args'] for e in known), 'explicit HiCar target was accidentally excluded'
            assert all('--exclude-peer' not in e['args'] for e in events() if e['event']=='start' and e['role']=='carplay'), 'HiCar exclusion leaked into CarPlay arguments'
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
            # A new boot clears only the session pause, then chooses the last
            # connected protocol even when a different HU-success record exists.
            ctl(op='shutdown');assert p.wait(timeout=12)==0;p=None
            stored=json.loads((root/'devices.json').read_text())
            for d in stored['devices']:d['auto']=True
            apple=next(d.copy() for d in stored['devices'] if d['protocol']=='carplay')
            huawei=next(d.copy() for d in stored['devices'] if d['protocol']=='hicar')
            apple['last_used']=100;stored.update(last=apple,preferred=huawei,selected_protocol='hicar',boot_protocol='last',paused=True,policy='last',boot_id='previous-test-boot')
            (root/'devices.json').write_text(json.dumps(stored));(root/'scenario').write_text('')
            p=subprocess.Popen([*runner,binary,'manager','run','--config',str(root/'config.json')],stdout=log,stderr=log)
            booted=until(lambda s:s['active'] and s['protocol']=='hicar')
            assert not booted['paused'], 'previous-boot pause suppressed automatic reconnect'
            # Two Huawei targets: the latest/explicit A is absent, B is present.
            # Rotate only within HiCar and preserve the USB owner. B then stays
            # connected past the initial deadline despite no new media reports.
            hc_other=dict(hc,address='02:00:00:00:00:04',name='other Huawei',last_used=1)
            ctl(op='remember',device=hc_other)
            (root/'scenario').write_text('hicar-first-offline')
            ctl(op='disconnect');ctl(op='policy',mode='last')
            marker=len(events())
            ctl(op='connect',protocol='hicar',address=hc['address'])
            absent=until(lambda s:s['protocol']=='hicar' and s['target'] and s['target']['address']==hc['address'] and not s['active'])
            recovered=until(lambda s:s['protocol']=='hicar' and s['active'] and s['target'] and s['target']['address']==hc_other['address'])
            assert recovered['carlife_pid']==absent['carlife_pid'], 'same-protocol rotation restarted USB'
            attempts=[e for e in events()[marker:] if e['event']=='start' and e['role']!='carlife']
            assert len(attempts)==2 and all(e['role']=='hicar' for e in attempts)
            assert [e['args'][e['args'].index('--peer')+1] for e in attempts]==[hc['address'],hc_other['address']]
            time.sleep(2.3)
            assert ctl(op='status')['status']['backend_pid']==recovered['backend_pid'], 'quiet active HiCar expired on the initial attempt timer'
            # A favorite must not use that working B as a fallback when A is away.
            ctl(op='policy',mode='favorite',protocol='hicar',address=hc['address'])
            ctl(op='disconnect');marker=len(events());ctl(op='resume')
            until(lambda s:s['backend_pid'] is not None and s['target'] and s['target']['address']==hc['address'])
            time.sleep(5)
            attempts=[e for e in events()[marker:] if e['event']=='start' and e['role']!='carlife']
            assert attempts and all(e['role']=='hicar' and e['args'][e['args'].index('--peer')+1]==hc['address'] for e in attempts), 'missing HiCar favorite fell back to B'
            # Forgetting the favorite releases its policy; resume and a cold
            # boot can select the other registered phone on the same protocol.
            ctl(op='forget',protocol='hicar',address=hc['address'],confirm=True)
            forgotten_store=json.loads((root/'devices.json').read_text())
            assert forgotten_store['policy']=='last' and forgotten_store['favorite'] is None
            (root/'scenario').write_text('');ctl(op='resume')
            until(lambda s:s['active'] and s['target'] and s['target']['address']==hc_other['address'])
            ctl(op='shutdown');assert p.wait(timeout=12)==0;p=None
            stored=json.loads((root/'devices.json').read_text());stored['boot_id']='previous-forget-boot'
            (root/'devices.json').write_text(json.dumps(stored))
            p=subprocess.Popen([*runner,binary,'manager','run','--config',str(root/'config.json')],stdout=log,stderr=log)
            until(lambda s:s['active'] and s['target'] and s['target']['address']==hc_other['address'])
            ctl(op='remember',device=hc)
            ctl(op='forget',protocol='hicar',address=hc_other['address'],confirm=True)
            (root/'scenario').write_text('')
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
            print(json.dumps(dict(connection_manager='passed',switches=9,usb_pid_preserved=True,target_only_bluetooth_disconnect=True,disconnect_failure_nonfatal=True,
                independent_usb_recovery=True,pre_usb_success_not_committed=True,radio_overlap=False,offline_pairing_retained=True,
                authentication_and_repeated_refusal_repaired=True,target_only=True,identity_unchanged=True,
                manual_and_favorite_policy=True,favorite_exact_target=True,favorite_forget_resume_and_boot=True,cross_protocol_pairing_reconciled=True,hicar_same_protocol_rotation=True,active_session_deadline_exempt=True,boot_reconnect_preference=True,current_auto_record_respected=True,hicar_enrollment_stability_gate=True,pairing_window_accepts_new_and_existing_devices=True,user_preferences_survive_repair_and_stability=True,forget_confirmation_and_persistence=True)))
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
