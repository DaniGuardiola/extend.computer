"""Run real source OS interaction checks through the paired, fixed-operation host."""
import argparse
import html
import json
import time
import uuid
from pathlib import Path
from bootstrap import ROOT
from client import Client


def execute(client, product, output, screen='auto'):
    output.mkdir(parents=True, exist_ok=False, mode=0o700)
    ident = str(uuid.uuid4())
    result = {'id': ident, 'status': 'failed', 'measurement_scope': 'Source interaction only'}
    started = False
    try:
        client.request('/context/' + product)
        reply = client.request('/interaction/start', {'id': ident, 'product': product, 'screen': screen})
        started = True
        if reply.get('id') != ident or reply.get('status') != 'running':
            raise RuntimeError('Source did not acknowledge the interaction fixture')
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            result = client.request('/interaction/state', {'id': ident})
            if result.get('id') != ident:
                raise RuntimeError('Interaction result ID mismatch')
            if result.get('status') != 'running':
                break
            time.sleep(.2)
        else:
            raise TimeoutError('Interaction fixture did not finish')
        if result.get('status') != 'complete':
            raise RuntimeError(result.get('error') or 'Source interaction failed')
        return result
    except BaseException as error:
        result = {**result, 'status': 'failed', 'error': str(error) or type(error).__name__}
        raise
    finally:
        if started:
            try:
                cleanup = client.request('/interaction/stop', {'id': ident})
                result['cleanup_status'] = cleanup.get('status')
            except Exception as error:
                result['cleanup_error'] = str(error)
                result['status'] = 'failed'
        (output / 'report.json').write_text(json.dumps(result, indent=2))
        rows = ''.join('<tr><td>' + html.escape(str(row.get('action'))) + '</td><td>' +
            html.escape(str(row.get('accepted'))) + '</td><td>' +
            html.escape(json.dumps(row.get('observed'))) + '</td></tr>'
            for row in (result.get('fixture') or {}).get('actions', []))
        (output / 'report.html').write_text('<meta charset="utf-8"><title>Source interaction checks</title>'
            '<style>body{font:16px system-ui;max-width:900px;margin:40px auto}td{padding:10px;border-bottom:1px solid #ddd}</style>'
            '<h1>Source interaction checks</h1><p>' + html.escape(result['status']) + '</p>'
            '<p>Real OS events and owned source controls. Receiver visual acceptance and physical input-to-photon latency are not measured.</p>'
            '<table><tr><th>Action</th><th>Accepted</th><th>Observed</th></tr>' + rows + '</table>'
            '<p>' + html.escape(result.get('error', '')) + '</p>')
        if result.get('cleanup_error'):
            raise RuntimeError('Source fixture cleanup could not be confirmed: ' + result['cleanup_error'])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--pairing', type=Path, default=ROOT / '.cache/campaign/source-pairing.json')
    parser.add_argument('--product', default='extend')
    parser.add_argument('--screen', default='auto')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = execute(Client(args.pairing), args.product, args.output.expanduser().resolve(), args.screen)
    print('Source interaction checks completed successfully. Results: ' + str(args.output / 'report.html'))


if __name__ == '__main__':
    main()
