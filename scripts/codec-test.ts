// Round-trip smoke test for the TOML codec (run with: npm run test:codec).
// Verifies DHCP encoding, [[peer]] parsing and key reversal logic.

import { encodeTOML, decodeTOML } from '../src/toml-codec';
import { defaultConfig } from '../src/network-config';
import { webcrypto } from 'node:crypto';
if (typeof (globalThis as Record<string, unknown>).crypto === 'undefined') {
  (globalThis as Record<string, unknown>).crypto = webcrypto as unknown as Crypto;
}

function assert(cond: boolean, msg: string) {
  if (!cond) { console.error('FAIL:', msg); process.exitCode = 1; } else { console.log('ok:', msg); }
}

// 1. DHCP on → must emit `dhcp = true` and no ipv4.
const dhcpOn = { ...defaultConfig(), dhcp: true, virtual_ipv4: '10.144.144.10' };
const toml1 = encodeTOML(dhcpOn);
assert(toml1.includes('dhcp = true'), 'dhcp = true is emitted when DHCP is on');
assert(!toml1.includes('ipv4 ='), 'ipv4 omitted when DHCP is on');

// 2. Round-trip of the user's real config (peers via [[peer]] array tables).
const userToml = `instance_name = "fn"
instance_id = "f88ec026-78bd-4e42-b3aa-4b7cce29dc23"
hostname = "T1"
listeners = ["tcp://0.0.0.0:11010", "udp://0.0.0.0:11010", "wg://0.0.0.0:11011"]

[network_identity]
network_name = "fn"
network_secret = "Goodnight2026@"

[[peer]]
uri = "txt://et.speedtest.6043443.xyz"

[[peer]]
uri = "tcp://225284.xyz:11010"

[flags]
latency_first = false
no_tun = true
need_p2p = true
proxy_forward_by_system = true
private_mode = true
dev_name = "et_16_9hou"
`;
const cfg2 = decodeTOML(userToml);
assert(cfg2.peer_urls.length === 2, `[[peer]] parsed (got ${cfg2.peer_urls.length}, want 2)`);
assert(cfg2.peer_urls[0] === 'txt://et.speedtest.6043443.xyz', 'peer uri #1 matches');
assert(cfg2.peer_urls[1] === 'tcp://225284.xyz:11010', 'peer uri #2 matches');
assert(cfg2.network_name === 'fn', 'network name decoded');
assert(cfg2.network_secret === 'Goodnight2026@', 'network secret decoded');
assert(cfg2.no_tun === true, 'no_tun flag decoded');
assert(cfg2.dhcp === true, 'missing ipv4+dhcp defaults to DHCP');

// 3. Explicit ipv4 wins over dhcp.
const toml3 = encodeTOML({ ...defaultConfig(), dhcp: false, virtual_ipv4: '10.10.1.100', network_length: 24 });
const cfg3 = decodeTOML(toml3);
assert(cfg3.dhcp === false && cfg3.virtual_ipv4 === '10.10.1.100' && cfg3.network_length === 24, 'manual ipv4 round-trips');

// 4. enable_ipv6 / enable_encryption reversal round-trips.
const cfg4 = decodeTOML(encodeTOML({ ...defaultConfig(), disable_ipv6: true, disable_encryption: true }));
assert(cfg4.disable_ipv6 === true && cfg4.disable_encryption === true, 'flag reversal round-trips');

// 5. Re-export of imported user config keeps peers.
assert(encodeTOML(cfg2).includes('txt://et.speedtest.6043443.xyz'), 're-export keeps peer uris');

if (!process.exitCode) console.log('\nall codec tests passed');
