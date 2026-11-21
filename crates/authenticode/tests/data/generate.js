// Authenticode test fixtures: a tiny PE32+ signed per Microsoft's
// "Windows Authenticode Portable Executable Signature Format" with a test
// PKI made by OpenSSL (3.4 or later). Every certificate has fixed validity
// dates, so the tests' clock (2026-11-02) is independent of when this ran.
//
//   node generate.js <output directory>
//
//   root.der                trust anchor (2024-01-01 .. 2044-01-01)
//   other-root.der          an unrelated root (chains to it fail)
//   signed.exe              valid signature, publisher "Doors Test Publisher"
//   tampered.exe            one code byte changed after signing
//   expired.exe             signer valid 2024 only, no timestamp
//   timestamped.exe         the 2024 signer with an RFC 3161 timestamp from
//                           2024-06-15: valid today
//   late-timestamp.exe      the 2024 signer stamped 2025-06-15, after its
//                           certificate expired
//   untrusted-timestamp.exe the 2024 signer stamped by a time-stamping
//                           authority under other-root (the stamp is ignored)
//   wrong-usage.exe         signer has only the serverAuth EKU
//   unsigned.exe            no certificate table
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const { execFileSync } = require('child_process');
const out = path.resolve(process.argv[2]);
fs.mkdirSync(out, { recursive: true });
const work = fs.mkdtempSync(path.join(require('os').tmpdir(), 'acgen-'));
const ssl = (...args) => execFileSync('openssl', args, { cwd: work, stdio: ['ignore', 'pipe', 'pipe'] });
const w = (name, text) => fs.writeFileSync(path.join(work, name), text);
// ---- PKI ----
const root = (name, cn, serial) => {
  w(name + '.cnf', `[req]\ndistinguished_name=dn\nprompt=no\n[dn]\nCN=${cn}\n[v3]\nbasicConstraints=critical,CA:true\nkeyUsage=critical,keyCertSign,cRLSign\nsubjectKeyIdentifier=hash\n`);
  ssl('req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', name + '.key', '-out', name + '.pem', '-config', name + '.cnf', '-extensions', 'v3', '-set_serial', String(serial), '-not_before', '20240101000000Z', '-not_after', '20440101000000Z');
};
root('root', 'Doors Test Root', 1);
root('other', 'Unrelated Root', 2);
let serial = 10;
function leaf(name, ca, cn, eku, from, to) {
  w(name + '.cnf', `[req]\ndistinguished_name=dn\nprompt=no\n[dn]\nCN=${cn}\nO=Doors Test Org\n[v3]\nbasicConstraints=critical,CA:false\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=critical,${eku}\n`);
  ssl('req', '-new', '-newkey', 'rsa:2048', '-nodes', '-keyout', name + '.key', '-out', name + '.csr', '-config', name + '.cnf');
  ssl('x509', '-req', '-in', name + '.csr', '-CA', ca + '.pem', '-CAkey', ca + '.key', '-out', name + '.pem', '-extfile', name + '.cnf', '-extensions', 'v3', '-set_serial', String(serial++), '-not_before', from, '-not_after', to);
}
leaf('signer', 'root', 'Doors Test Publisher', 'codeSigning', '20250101000000Z', '20350101000000Z');
leaf('expired', 'root', 'Doors Expired Publisher', 'codeSigning', '20240101000000Z', '20241231235959Z');
leaf('server', 'root', 'Doors Server Only', 'serverAuth', '20250101000000Z', '20350101000000Z');
leaf('tsa', 'root', 'Doors Test Time Stamping', 'timeStamping', '20240101000000Z', '20340101000000Z');
leaf('rogue-tsa', 'other', 'Unrelated Time Stamping', 'timeStamping', '20240101000000Z', '20340101000000Z');
const derOf = (pem) => Buffer.from(fs.readFileSync(path.join(work, pem), 'utf8').replace(/-----[^-]+-----/g, '').replace(/\s+/g, ''), 'base64');
const keyOf = (name) => crypto.createPrivateKey(fs.readFileSync(path.join(work, name + '.key')));
fs.writeFileSync(path.join(out, 'root.der'), derOf('root.pem'));
fs.writeFileSync(path.join(out, 'other-root.der'), derOf('other.pem'));
// ---- DER helpers ----
function len(n) {
  if (n < 0x80) return Buffer.from([n]);
  const b = [];
  while (n > 0) { b.unshift(n & 0xff); n >>= 8; }
  return Buffer.from([0x80 | b.length, ...b]);
}
const tlv = (tag, ...parts) => { const body = Buffer.concat(parts); return Buffer.concat([Buffer.from([tag]), len(body.length), body]); };
const seq = (...p) => tlv(0x30, ...p);
const set = (...p) => tlv(0x31, ...p);
const oid = (dotted) => {
  const n = dotted.split('.').map(Number);
  const bytes = [40 * n[0] + n[1]];
  for (const v of n.slice(2)) {
    const enc = [v & 0x7f];
    let x = v >> 7;
    while (x > 0) { enc.unshift(0x80 | (x & 0x7f)); x >>= 7; }
    bytes.push(...enc);
  }
  return tlv(0x06, Buffer.from(bytes));
};
const nul = Buffer.from([0x05, 0x00]);
const int = (n) => tlv(0x02, Buffer.from([n]));
const octets = (b) => tlv(0x04, b);
const ctx = (n, ...p) => tlv(0xa0 | n, ...p);
const sha256 = (b) => crypto.createHash('sha256').update(b).digest();
const SHA256 = () => seq(oid('2.16.840.1.101.3.4.2.1'), nul);
const RSA = () => seq(oid('1.2.840.113549.1.1.1'), nul);
// A certificate's IssuerAndSerialNumber.
function issuerSerial(der) {
  const read = (b, at) => {
    const tag = b[at];
    let l = b[at + 1], h = 2;
    if (l & 0x80) { const k = l & 0x7f; l = 0; for (let i = 0; i < k; i++) l = (l << 8) | b[at + 2 + i]; h = 2 + k; }
    return { tag, start: at, body: at + h, end: at + h + l };
  };
  const cert = read(der, 0);
  const tbs = read(der, cert.body);
  let at = tbs.body;
  let f = read(der, at);
  if (f.tag === 0xa0) { at = f.end; f = read(der, at); }
  const serial = der.slice(f.start, f.end);
  at = f.end;
  const sigalg = read(der, at); at = sigalg.end;
  const issuer = read(der, at);
  return seq(der.slice(issuer.start, issuer.end), serial);
}
// A SignerInfo over `attrSet` (signed attributes, DER SET) by `name`.
function signerInfo(name, attrSet, unauthenticated) {
  const signature = crypto.sign('sha256', attrSet, keyOf(name));
  const parts = [int(1), issuerSerial(derOf(name + '.pem')), SHA256(), Buffer.concat([Buffer.from([0xa0]), attrSet.slice(1)]), RSA(), octets(signature)];
  if (unauthenticated) parts.push(Buffer.concat([Buffer.from([0xa1]), unauthenticated.slice(1)]));
  return { info: seq(...parts), signature };
}
// An RFC 3161 time-stamp token (ContentInfo) over `signature` at `when`.
function timestampToken(tsa, signature, when) {
  const tst = seq(
    int(1),
    oid('1.3.6.1.4.1.311.99.1'),
    seq(SHA256(), octets(sha256(signature))),
    int(7),
    tlv(0x18, Buffer.from(when, 'latin1')),
  );
  const attrs = set(
    seq(oid('1.2.840.113549.1.9.3'), set(oid('1.2.840.113549.1.9.16.1.4'))),
    seq(oid('1.2.840.113549.1.9.4'), set(octets(sha256(tst)))),
  );
  const signedData = seq(
    int(3),
    set(SHA256()),
    seq(oid('1.2.840.113549.1.9.16.1.4'), ctx(0, octets(tst))),
    ctx(0, derOf(tsa + '.pem')),
    set(signerInfo(tsa, attrs).info),
  );
  return seq(oid('1.2.840.113549.1.7.2'), ctx(0, signedData));
}
// ---- PE ----
function pe() {
  const b = Buffer.alloc(0x400);
  b.write('MZ', 0);
  b.writeUInt32LE(0x80, 0x3c);
  b.write('PE\0\0', 0x80);
  const coff = 0x84;
  b.writeUInt16LE(0x8664, coff);
  b.writeUInt16LE(1, coff + 2);
  b.writeUInt16LE(240, coff + 16);
  b.writeUInt16LE(0x22, coff + 18);
  const opt = 0x98;
  b.writeUInt16LE(0x20b, opt);
  b.writeUInt32LE(0x200, opt + 4);
  b.writeUInt32LE(0x1000, opt + 16);
  b.writeBigUInt64LE(0x140000000n, opt + 24);
  b.writeUInt32LE(0x1000, opt + 32);
  b.writeUInt32LE(0x200, opt + 36);
  b.writeUInt16LE(6, opt + 40);
  b.writeUInt32LE(0x2000, opt + 56);
  b.writeUInt32LE(0x200, opt + 60);
  b.writeUInt16LE(3, opt + 68);
  b.writeUInt32LE(16, opt + 108);
  const sec = opt + 240;
  b.write('.text', sec);
  b.writeUInt32LE(0x10, sec + 8);
  b.writeUInt32LE(0x1000, sec + 12);
  b.writeUInt32LE(0x200, sec + 16);
  b.writeUInt32LE(0x200, sec + 20);
  b.writeUInt32LE(0x60000020, sec + 36);
  b.write('\xc3Doors test code', 0x200, 'latin1');
  return b;
}
const CHECKSUM = 0x98 + 64;
const SECDIR = 0x98 + 112 + 4 * 8;
function digest(file, tableAt) {
  const h = crypto.createHash('sha256');
  h.update(file.slice(0, CHECKSUM));
  h.update(file.slice(CHECKSUM + 4, SECDIR));
  h.update(file.slice(SECDIR + 8, tableAt));
  return h.digest();
}
// Sign `image` as `signer`; `stamp` = [tsa, GeneralizedTime] adds an
// RFC 3161 timestamp (szOID_RFC3161_counterSign unsigned attribute).
function sign(image, signer, stamp) {
  const file = Buffer.from(image);
  const tableAt = file.length; // already 8-aligned
  const spc = seq(
    seq(oid('1.3.6.1.4.1.311.2.1.15'), seq(tlv(0x03, Buffer.from([0])), ctx(0, ctx(2, tlv(0x80, Buffer.alloc(0)))))),
    seq(SHA256(), octets(digest(file, tableAt))),
  );
  const spcContents = spc.slice(2 + (spc[1] & 0x80 ? spc[1] & 0x7f : 0));
  const attrSet = set(
    seq(oid('1.2.840.113549.1.9.3'), set(oid('1.3.6.1.4.1.311.2.1.4'))),
    seq(oid('1.2.840.113549.1.9.4'), set(octets(sha256(spcContents)))),
  );
  let info = signerInfo(signer, attrSet).info;
  if (stamp) {
    const { signature } = signerInfo(signer, attrSet);
    const unauth = set(seq(oid('1.3.6.1.4.1.311.3.3.1'), set(timestampToken(stamp[0], signature, stamp[1]))));
    info = signerInfo(signer, attrSet, unauth).info;
  }
  const signedData = seq(
    int(1),
    set(SHA256()),
    seq(oid('1.3.6.1.4.1.311.2.1.4'), ctx(0, spc)),
    ctx(0, derOf(signer + '.pem'), derOf('root.pem')),
    set(info),
  );
  const content = seq(oid('1.2.840.113549.1.7.2'), ctx(0, signedData));
  let entry = Buffer.alloc(8);
  entry.writeUInt16LE(0x0200, 4);
  entry.writeUInt16LE(0x0002, 6);
  entry = Buffer.concat([entry, content]);
  const padded = Buffer.concat([entry, Buffer.alloc((8 - (entry.length % 8)) % 8)]);
  padded.writeUInt32LE(padded.length, 0);
  file.writeUInt32LE(tableAt, SECDIR);
  file.writeUInt32LE(padded.length, SECDIR + 4);
  return Buffer.concat([file, padded]);
}
const image = pe();
const put = (name, bytes) => fs.writeFileSync(path.join(out, name), bytes);
put('unsigned.exe', image);
const signed = sign(image, 'signer');
put('signed.exe', signed);
const tampered = Buffer.from(signed);
tampered[0x205] ^= 1;
put('tampered.exe', tampered);
put('expired.exe', sign(image, 'expired'));
put('timestamped.exe', sign(image, 'expired', ['tsa', '20240615120000Z']));
put('late-timestamp.exe', sign(image, 'expired', ['tsa', '20250615120000Z']));
put('untrusted-timestamp.exe', sign(image, 'expired', ['rogue-tsa', '20240615120000Z']));
put('wrong-usage.exe', sign(image, 'server'));
fs.rmSync(work, { recursive: true, force: true });
console.log('fixtures in', out);
