#!/usr/bin/env python3
"""Start the built app headless in a folder with a non-ASCII name and check the basics end to end:
first run, a password-protected tree (the file must not be plain SQLite), lock, wrong and right password,
agent door refused while locked. Used by the Windows CI job; also runs on Linux/macOS.

  python3 tools/app-smoke.py path/to/family-app[.exe]
"""
import json, os, subprocess, sys, tempfile, time, urllib.request, urllib.error


def call(port, method, path, body=None, headers=None):
    h = {'Sec-Fetch-Site': 'same-origin', 'Host': f'127.0.0.1:{port}'}
    h.update(headers or {})
    data = None
    if body is not None:
        data = json.dumps(body).encode()
        h['Content-Type'] = 'application/json'
    req = urllib.request.Request(f'http://127.0.0.1:{port}{path}', data=data, method=method, headers=h)
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            return r.status, json.loads(r.read() or b'null')
    except urllib.error.HTTPError as e:
        try:
            return e.code, json.loads(e.read() or b'null')
        except Exception:
            return e.code, None


def main():
    exe = os.path.abspath(sys.argv[1])
    base = tempfile.mkdtemp(prefix='fq-smoke-')
    data = os.path.join(base, 'خانواده tree')  # Persian letters and a space, as a real user folder may have
    port = 18700 + os.getpid() % 200
    p = subprocess.Popen([exe, '--headless', '--data', data, '--port', str(port), '--fast-kdf-for-tests'], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        for _ in range(100):
            try:
                st, v = call(port, 'GET', '/api/tree')
                if st == 200:
                    break
            except Exception:
                time.sleep(0.2)
        else:
            raise SystemExit('the app did not start')
        assert v['state'] == 'missing', v
        st, v = call(port, 'POST', '/api/tree/create', {'title': 'درخت خانواده', 'password': 'correct horse battery'})
        assert st == 200 and v['recovery_key'], (st, v)
        st, v = call(port, 'GET', '/api/tree')
        assert v['state'] == 'open' and v['protected'] and v['title'] == 'درخت خانواده', v
        with open(os.path.join(data, 'tree.sqlite'), 'rb') as f:
            assert f.read(16) != b'SQLite format 3\x00', 'the protected file is plain SQLite'
        st, v = call(port, 'POST', '/api/people', {'given_name': 'Ada', 'family_name': 'Root', 'sex': 'F'})
        assert st in (200, 201), (st, v)
        assert call(port, 'POST', '/api/tree/lock', {})[0] == 200
        assert call(port, 'GET', '/api/people?q=Ada')[0] == 423
        token = open(os.path.join(data, 'agent-token.txt')).read().strip()
        assert call(port, 'GET', '/agent/v1/people?q=Ada', headers={'Authorization': f'Bearer {token}'})[0] == 423
        st, v = call(port, 'POST', '/api/tree/unlock', {'password': 'nope'})
        assert st == 423 and v['error']['code'] == 'wrong_password', (st, v)
        assert call(port, 'POST', '/api/tree/unlock', {'password': 'correct horse battery'})[0] == 200
        st, v = call(port, 'GET', '/api/people?q=Ada')
        assert st == 200 and len(v['people']) == 1, (st, v)
        print('app smoke test: all passed')
    finally:
        p.terminate()
        try:
            p.wait(timeout=10)
        except Exception:
            p.kill()


main()
