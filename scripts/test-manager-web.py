"""R3 maintenance HTTP boundary tests against the real manager process.

Synthetic process commands and temporary sockets only; no board/radio access.
The application itself is built exclusively by GitHub Actions.
"""
import contextlib
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import threading
import time


def parse_response(response):
    assert response, 'HTTP peer closed without a reply'
    assert b'\r\n\r\n' in response, 'HTTP reply header was truncated'
    head,body=response.split(b'\r\n\r\n',1)
    lines=head.decode('ascii').split('\r\n')
    code=int(lines[0].split()[1])
    headers=dict(line.split(': ',1) for line in lines[1:])
    assert int(headers['Content-Length'])==len(body), 'HTTP reply body was truncated or oversized'
    assert headers['Connection']=='close'
    assert headers['Cache-Control']=='no-store'
    assert headers['X-Content-Type-Options']=='nosniff'
    return code,headers,body


def receive_response(stream):
    response=b''
    while True:
        try:
            part=stream.recv(65536)
        except ConnectionResetError:
            # Rejections can close with unread request data. A reset is EOF
            # only after a complete declared response has already arrived.
            return parse_response(response)
        if not part:break
        response+=part
        assert len(response)<=131072, 'unbounded HTTP response'
    return parse_response(response)


def check(binary):
    runner = json.loads(os.environ.get('CARLINK_RUST_RUNNER', '[]'))
    with tempfile.TemporaryDirectory(prefix='carlink-manager-web-') as directory:
        root = Path(directory)
        with socket.socket() as reservation:
            reservation.bind(('127.0.0.1', 0))
            port = reservation.getsockname()[1]
        authority = f'127.0.0.1:{port}'
        config = dict(schema=1, socket=str(root/'control.sock'),
            live_socket=str(root/'live.sock'), state=str(root/'devices.json'),
            radio_lock=str(root/'radio.lock'), media_socket=str(root/'media.sock'),
            carlife=[sys.executable, '-c', 'import time; time.sleep(90)'],
            carplay=[], hicar=[], web_listen=authority,
            devices=[], connect_seconds=2, stable_seconds=1, pair_seconds=5)
        path=root/'config.json'
        path.write_text(json.dumps(config), encoding='utf-8');path.chmod(0o600)
        log=(root/'manager.log').open('wb')
        process=None
        def ctl(command):
            with socket.socket(socket.AF_UNIX) as stream:
                stream.settimeout(12);stream.connect(str(root/'control.sock'))
                stream.sendall(json.dumps(command).encode()+b'\n')
                response=b''
                while not response.endswith(b'\n'):
                    part=stream.recv(32768)
                    if not part:raise AssertionError('manager control closed without a reply')
                    response+=part
            return json.loads(response)
        def raw(request):
            with socket.create_connection(('127.0.0.1',port),timeout=3) as stream:
                stream.settimeout(5);stream.sendall(request)
                return receive_response(stream)
        def get(path,host=authority):
            return raw(f'GET {path} HTTP/1.1\r\nHost: {host}\r\n\r\n'.encode())
        def post(command,origin=None,host=authority,kind='application/json'):
            body=json.dumps(command).encode()
            origin=f'http://{host}' if origin is None else origin
            return raw((f'POST /api/control HTTP/1.1\r\nHost: {host}\r\nOrigin: {origin}\r\n'
                f'Content-Type: {kind}\r\nContent-Length: {len(body)}\r\n\r\n').encode()+body)
        @contextlib.contextmanager
        def drip():
            # Keep each byte below the socket's idle timeout, while the whole
            # valid JSON body takes much longer than the total request budget.
            stream=socket.create_connection(('127.0.0.1',port),timeout=3)
            stream.settimeout(5)
            payload=b'{"op":"pause"}'
            headers=(f'POST /api/control HTTP/1.1\r\nHost: {authority}\r\n'
                f'Origin: http://{authority}\r\nContent-Type: application/json\r\n'
                f'Content-Length: {len(payload)}\r\n\r\n').encode()
            stream.sendall(headers)
            stop=threading.Event();sent=[]
            def send():
                for byte in payload:
                    if stop.is_set():break
                    try:stream.sendall(bytes([byte]))
                    except OSError:break
                    sent.append(time.monotonic())
                    if stop.wait(.4):break
            started=time.monotonic()
            sender=threading.Thread(target=send,daemon=True);sender.start()
            try:
                yield stream,started,sent
            finally:
                stop.set();sender.join(timeout=2);stream.close()
                assert not sender.is_alive(), 'drip sender did not stop'
        try:
            # Failed startup must undo its private control endpoint so the same
            # configuration succeeds immediately after the occupied port clears.
            with socket.socket() as occupied:
                occupied.bind(('127.0.0.1',port));occupied.listen(1)
                failed=subprocess.Popen([*runner,binary,'manager','run','--config',str(path)],stdout=log,stderr=log)
                try:
                    assert failed.wait(timeout=8)!=0, 'manager ignored an occupied maintenance port'
                finally:
                    if failed.poll() is None:failed.kill();failed.wait(timeout=5)
                assert not os.path.lexists(root/'control.sock'), 'failed startup left a stale control endpoint'
            process=subprocess.Popen([*runner,binary,'manager','run','--config',str(path)],stdout=log,stderr=log)
            deadline=time.monotonic()+15
            while time.monotonic()<deadline:
                try:
                    if get('/api/status')[0]==200:break
                except (OSError,ValueError):pass
                if process.poll() is not None:raise AssertionError('maintenance manager exited at startup')
                time.sleep(.05)
            else:raise AssertionError('maintenance HTTP listener did not become ready')
            code,headers,body=get('/')
            assert code==200 and headers['Content-Type'].startswith('text/html') and b'CarLink' in body
            assert b"frame-ancestors 'none'" in headers['Content-Security-Policy'].encode()
            for route,kind in [('/app.js','application/javascript'),('/style.css','text/css')]:
                code,headers,body=get(route)
                assert code==200 and headers['Content-Type'].startswith(kind) and body
            code,_,body=get('/api/status')
            assert code==200 and json.loads(body)['ok']
            code,_,body=get('/api/devices')
            assert code==200 and json.loads(body)==dict(ok=True,devices=[])
            assert get('/api/status',host=f'carlink.local:{port}')[0]==200
            code,_,body=post(dict(op='pause'))
            assert code==200 and json.loads(body)['status']['paused']
            code,_,body=post(dict(op='resume'))
            assert code==200 and not json.loads(body)['status']['paused']
            # Web allowlist excludes process lifecycle and unrestricted record
            # injection even when the browser otherwise satisfies same-origin.
            for operation in ('shutdown','remember','repair','shell','__proto__'):
                code,_,body=post(dict(op=operation))
                assert code==200 and json.loads(body)['ok'] is False, operation
            assert ctl(dict(op='status'))['ok'], 'rejected web op killed the manager'
            rejected=[
                f'GET /api/status HTTP/1.1\r\nHost: evil.example:{port}\r\n\r\n'.encode(),
                b'GET /api/status HTTP/1.1\r\n\r\n',
                (f'POST /api/control HTTP/1.1\r\nHost: {authority}\r\nOrigin: http://{authority}\r\n'
                 'Content-Type: application/json\r\nContent-Length: 2\r\ncontent-length: 2\r\n\r\n{}').encode(),
                (f'POST /api/control HTTP/1.1\r\nHost: {authority}\r\nOrigin: http://{authority}\r\n'
                 'Content-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n').encode(),
                (f'POST /api/control HTTP/1.1\r\nHost: {authority}\r\nOrigin: http://{authority}\r\n'
                 'Content-Type: application/json\r\nContent-Length: 16385\r\n\r\n').encode(),
                (f'POST /api/control HTTP/1.1\r\nHost: {authority}\r\n'
                 'Content-Type: application/json\r\nContent-Length: 2\r\n\r\n{}').encode(),
                (f'GET /api/status HTTP/1.1\r\nHost: {authority}\r\nContent-Length: 1\r\n\r\nx').encode(),
            ]
            for request in rejected:
                code,_,_=raw(request)
                assert code==400, 'malformed or untrusted request was accepted'
            assert post(dict(op='pause'),origin='https://evil.example')[0]==400
            assert post(dict(op='pause'),kind='text/plain')[0]==400
            assert not ctl(dict(op='status'))['status']['paused'], 'rejected request mutated connection state'
            assert get('/api/../config.json')[0]==404
            assert get('/api/status')[0]==200, 'listener did not recover after rejected requests'
            with drip() as (stream,started,sent):
                code,_,_=receive_response(stream)
                finished=time.monotonic()
                assert code==400, 'dripped body escaped the total request deadline'
                assert finished-started<4, 'dripped body monopolized the HTTP listener'
                assert len(sent)>=3 and max(b-a for a,b in zip(sent,sent[1:]))<.65 and finished-sent[-1]<.65, 'drip fixture hit the idle timeout instead of exercising the total deadline'
            assert get('/api/status')[0]==200, 'listener did not recover after a dripped body'
            assert not ctl(dict(op='status'))['status']['paused'], 'partial dripped JSON mutated state'
            # A second in-flight body also bounds Web::drop during shutdown.
            with drip():
                time.sleep(.15)
                started=time.monotonic()
                ctl(dict(op='shutdown'));assert process.wait(timeout=5)==0;process=None
                assert time.monotonic()-started<4.5, 'shutdown waited for the dripped body to finish'
            assert not os.path.lexists(root/'control.sock'), 'normal shutdown left a control endpoint'
            print(json.dumps(dict(manager_web='passed',http_requests=True,same_origin_control=True,
                duplicate_length_and_chunked_rejected=True,body_bound=True,operation_allowlist=True,
                body_drip_total_deadline=True,startup_failure_cleanup=True,bounded_shutdown=True)))
        except BaseException:
            log.flush()
            print((root/'manager.log').read_text(encoding='utf-8',errors='replace')[-8000:],file=sys.stderr)
            raise
        finally:
            if process and process.poll() is None:
                process.terminate()
                try:process.wait(timeout=12)
                except subprocess.TimeoutExpired:process.kill();process.wait(timeout=5)
            log.close()


if __name__=='__main__':
    check(str(Path(sys.argv[1]).resolve()))
