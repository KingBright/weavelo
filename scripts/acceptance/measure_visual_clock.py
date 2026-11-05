#!/usr/bin/env python3
"""Sample an explicitly selected same-host visual echo through native CUA.

The clock must originate on this computer and return through another viewer.
This avoids cross-host clock synchronization. Results include both media hops
and the return decoder checkpoint. Physical glass-to-glass is not measured.
"""
import argparse
import base64
import json
import math
from pathlib import Path
import select
import statistics
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--decoder', type=Path, required=True)
    parser.add_argument('--target-json', required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--samples', type=int, default=8, choices=range(3, 61))
    args = parser.parse_args()
    target = json.loads(args.target_json)
    assert isinstance(target.get('source', {}).get('Window'), int), 'Explicit clock-return window required'
    assert target.get('peer_device_id') and target.get('connection_id'), 'Exact connection required'
    args.output.mkdir(parents=True, exist_ok=True)
    child = subprocess.Popen([str(args.binary.resolve()), '--mcp-server'], stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=(args.output / 'observer.stderr').open('w'), text=True, bufsize=1)
    sequence = 0

    def rpc(method, params):
        nonlocal sequence
        sequence += 1
        child.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': sequence, 'method': method, 'params': params}) + '\n')
        child.stdin.flush()
        if not select.select([child.stdout], [], [], 15)[0]:
            raise TimeoutError('Observe original observer process; do not replay')
        value = json.loads(child.stdout.readline())
        assert value['id'] == sequence and 'result' in value, value
        return value['result']

    samples = []
    wall_origin = time.time_ns()
    monotonic_origin = time.monotonic_ns()
    try:
        rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {},
            'clientInfo': {'name': 'weavelo-owned-visual-clock', 'version': '1'}})
        for index in range(args.samples):
            start = time.time_ns() / 1_000_000
            monotonic = time.monotonic_ns() / 1_000_000
            observed = rpc('tools/call', {'name': 'weavelo_cua_observe',
                'arguments': {'target': target, 'control': False}})
            end = time.time_ns() / 1_000_000
            elapsed = time.monotonic_ns() / 1_000_000 - monotonic
            assert abs(end - start - elapsed) < 2, 'Local wall clock stepped during sampling'
            assert abs((time.time_ns() - wall_origin) -
                (time.monotonic_ns() - monotonic_origin)) < 2_000_000, 'Local wall clock stepped between samples'
            assert not observed.get('isError'), observed
            metadata = next(json.loads(block['text']) for block in observed['content']
                if block.get('type') == 'text' and block.get('text', '').startswith('{'))
            decoded_at = metadata.get('decoded_at_utc_us')
            assert isinstance(decoded_at, int), 'Observer needs the local decoded-frame timestamp'
            images = [block for block in observed['content'] if block.get('type') == 'image']
            assert len(images) == 1 and images[0]['mimeType'] == 'image/png'
            image = args.output / 'current-marker.png'
            image.write_bytes(base64.b64decode(images[0]['data'], validate=True))
            decoded = json.loads(subprocess.check_output([str(args.decoder.resolve()), str(image)], text=True, timeout=15))
            stamp = decoded['draw_utc_ms']
            assert 0 <= end - stamp < 30_000, 'Wrong clock origin, clock step, or stale marker'
            age = decoded_at / 1000 - stamp
            assert -1 <= age < 30_000, 'Clock origin or decoded-frame timestamp mismatch'
            sample = {'sample': index, 'source_draw_utc_ms': stamp,
                'return_decoded_at_utc_us': decoded_at, 'draw_to_return_decode_ms': max(0, age),
                'request_start_utc_ms': start, 'response_end_utc_ms': end,
                'age_lower_ms': max(0, start - stamp), 'age_upper_ms': end - stamp,
                'observer_bracket_ms': end - start}
            samples.append(sample)
            print(json.dumps(sample), flush=True)
            time.sleep(.25)
    finally:
        child.stdin.close()
        try:
            child.wait(timeout=4)
        except subprocess.TimeoutExpired:
            child.terminate(); child.wait(timeout=4)
        (args.output / 'samples.json').write_text(json.dumps(samples, indent=2) + '\n')
    assert len(samples) == args.samples
    def percentile(key, fraction):
        values = sorted(row[key] for row in samples)
        return statistics.median(values) if fraction == .5 else values[math.ceil(len(values) * fraction) - 1]
    report = {'measurement': 'same-host source draw to two-hop return decoded texture',
        'media_hops': 2, 'physical_presentation_measured': False, 'one_way_latency_measured': False,
        'samples': len(samples), 'p50_ms': percentile('draw_to_return_decode_ms', .5),
        'p95_ms': percentile('draw_to_return_decode_ms', .95), 'source_marker_quantization_ms': 1,
        'note': 'Includes both media paths and intermediate composition/capture; do not divide by two to claim one-way latency.'}
    (args.output / 'report.json').write_text(json.dumps(report, indent=2) + '\n')
    print(json.dumps(report), flush=True)


if __name__ == '__main__':
    main()
