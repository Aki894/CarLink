"""Rust/Python native PAKE and persisted STS interoperability; synthetic only."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT/'experiments/hicar'))


def python_worker():
    from hichain import Runtime,Session
    config=json.loads(sys.stdin.readline())
    runtime=Runtime(config['library'],config['store'],config['hardwareSeed'])
    session=None
    print(json.dumps(dict(ready=True)),flush=True)
    try:
        for line in sys.stdin:
            command=json.loads(line)
            if command['op']=='close':
                print(json.dumps(dict(closed=True)),flush=True);break
            if command['op']=='begin':
                session=Session(runtime,config['own'],config['peer'],config['pin'],role=config['role'],
                                package='com.huawei.hicar',service='CarDevice')
                code,packets=((session.authenticate(16) if command['phase']=='auth' else session.start())
                              if config['role']==1 else (0,[]))
            else:
                code,packets=session.receive(command['data'].encode())
            key=session.session_key
            print(json.dumps(dict(code=code,packets=[p.decode() for p in packets],results=session.results,
                 key_hash=hashlib.sha256(key).hexdigest() if key else None,key_length=len(key) if key else None)),flush=True)
    finally:
        if session:session.close()


def check(binary,library,rust_library=None):
    spec=importlib.util.spec_from_file_location('hicar_diff',ROOT/'scripts/test-hicar-rust.py')
    module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
    Process=module.Process
    rust_command=[*json.loads(os.environ.get('CARLINK_RUST_RUNNER','[]')),binary,'hicar','auth-worker']
    rust_library=rust_library or library
    cases=[]
    # Trusted auth deliberately loads the other implementation's persisted
    # store after both processes have exited. No export/re-pair workaround.
    for initial,reconnect in [('RP','PR'),('PR','RP'),('RR','RR')]:
        with tempfile.TemporaryDirectory(prefix='carlink-rust-auth-') as directory:
            for phase,implementations in [('bind',initial),('auth',reconnect)]:
                processes=[]
                try:
                    for index,implementation in enumerate(implementations):
                        own,peer=('AB'*32,'CD'*32) if index==0 else ('CD'*32,'AB'*32)
                        p=Process(rust_command if implementation=='R'
                                  else [sys.executable,__file__,'--python-worker'])
                        processes.append(p)
                        result=p.call(dict(library=rust_library if implementation=='R' else library,store=str(Path(directory)/str(index)),
                                hardwareSeed=own,own=own,peer=peer,pin='012345',role=index+1),timeout=30)
                        assert result.get('ready') is True,'native ABI/store initialization failed'
                    states=[None,processes[1].call(dict(op='begin',phase=phase),timeout=30)]
                    states[0]=processes[0].call(dict(op='begin',phase=phase),timeout=30)
                    pending=[(1,p) for p in states[0]['packets']];messages=[]
                    for _ in range(32):
                        if not pending:break
                        target,packet=pending.pop(0)
                        messages.append(json.loads(packet)['message'])
                        state=processes[target].call(dict(op='receive',data=packet),timeout=30)
                        assert 'error' not in state,'native worker reported error'
                        assert 1 not in state['results'],'native auth rejected synthetic correct PIN'
                        states[target]=state
                        pending.extend((1-target,p) for p in state['packets'])
                    assert not pending,'unbounded native authentication exchange'
                    assert all(0 in s['results'] for s in states),'native authentication incomplete'
                    assert states[0]['key_hash']==states[1]['key_hash'],'session keys differ'
                    assert all(s['key_length']==(32 if phase=='bind' else 16) for s in states),'wrong key length'
                    cases.append(dict(phase=phase,implementations=implementations,messages=messages,
                                      keys_match=True,persisted_other_implementation=initial!=reconnect and phase=='auth'))
                finally:
                    for p in processes:
                        try:p.call(dict(op='close'))
                        except (BrokenPipeError,AssertionError):pass
                        p.close()
    # A second fresh phone must bind to the same car identity/store without
    # replacing the first phone's keys. Reconnect both, then the first again.
    with tempfile.TemporaryDirectory(prefix='carlink-rust-multipeer-') as directory:
        root=Path(directory)
        car='AB'*32
        for phase,phone_index in [('bind',0),('bind',1),('auth',0),('auth',1),('auth',0)]:
            phone=('CD' if phone_index==0 else 'EF')*32
            processes=[]
            try:
                for index,(own,peer,store) in enumerate([(phone,car,root/('phone-'+str(phone_index))),
                                                       (car,phone,root/'car')]):
                    process=Process(rust_command);processes.append(process)
                    assert process.call(dict(library=rust_library,store=str(store),hardwareSeed=own,
                        own=own,peer=peer,pin='012345',role=index+1),timeout=30)['ready']
                states=[None,processes[1].call(dict(op='begin',phase=phase),timeout=30)]
                states[0]=processes[0].call(dict(op='begin',phase=phase),timeout=30)
                pending=[(1,packet) for packet in states[0]['packets']]
                for _ in range(32):
                    if not pending:break
                    target,packet=pending.pop(0)
                    states[target]=processes[target].call(dict(op='receive',data=packet),timeout=30)
                    assert 'error' not in states[target] and 1 not in states[target]['results'], 'shared-car-store authentication failed'
                    pending.extend((1-target,packet) for packet in states[target]['packets'])
                assert not pending and all(0 in state['results'] for state in states)
                assert states[0]['key_hash']==states[1]['key_hash']
                assert all(state['key_length']==(32 if phase=='bind' else 16) for state in states)
                cases.append(dict(phase=phase,implementations='RR',shared_car_store=True,
                                  phone_index=phone_index,keys_match=True))
            finally:
                for process in processes:
                    try:process.call(dict(op='close'))
                    except (BrokenPipeError,AssertionError):pass
                    process.close()
    # Wrong PIN must fail without a service key; no packet bodies are printed.
    with tempfile.TemporaryDirectory(prefix='carlink-rust-badpin-') as directory:
        peers=[]
        try:
            for index in (0,1):
                own,peer=('AB'*32,'CD'*32) if index==0 else ('CD'*32,'AB'*32)
                p=Process(rust_command);peers.append(p)
                assert p.call(dict(library=rust_library,store=str(Path(directory)/str(index)),hardwareSeed=own,
                    own=own,peer=peer,pin='012345' if index==0 else '987654',role=index+1))['ready']
            states=[None,peers[1].call(dict(op='begin',phase='bind'))]
            states[0]=peers[0].call(dict(op='begin',phase='bind'))
            pending=[(1,p) for p in states[0]['packets']]
            rejected=False
            for _ in range(24):
                if not pending:break
                target,packet=pending.pop(0)
                states[target]=peers[target].call(dict(op='receive',data=packet))
                rejected|=1 in states[target].get('results',[]) or 'error' in states[target]
                if rejected:break
                pending.extend((1-target,p) for p in states[target]['packets'])
            assert rejected,'wrong PIN was not rejected'
            assert all(s.get('key_hash') is None for s in states),'wrong PIN produced a service key'
        finally:
            for p in peers:
                try:p.call(dict(op='close'))
                except (BrokenPipeError,AssertionError):pass
                p.close()
    print(json.dumps(dict(hicar_rust_native='passed',cases=cases,wrong_pin_rejected=True,
                          raw_credentials_logged=False)))


if __name__=='__main__':
    if sys.argv[1:] == ['--python-worker']:
        python_worker()
    else:
        check(str(Path(sys.argv[1]).resolve()),str(Path(sys.argv[2]).resolve()),
              str(Path(sys.argv[3]).resolve()) if len(sys.argv)>3 else None)
