"""Rewrite `protocol_admin` of the cloned CofferPoolConfig (offset 8..40) to the given wallet."""
import base64, json, pathlib, sys
ALPHABET = b'123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
def b58decode(s):
    n = 0
    for c in s.encode(): n = n * 58 + ALPHABET.index(c)
    full = n.to_bytes((n.bit_length() + 7) // 8, 'big') if n else b''
    return b'\0' * (len(s) - len(s.lstrip('1'))) + full
path = pathlib.Path(__file__).parent / 'config.E9K7.local.json'
j = json.load(open(path))
data = bytearray(base64.b64decode(j['account']['data'][0]))
data[8:40] = b58decode(sys.argv[1])
j['account']['data'][0] = base64.b64encode(bytes(data)).decode()
json.dump(j, open(path, 'w'), indent=1)
print('config protocol_admin ->', sys.argv[1])
