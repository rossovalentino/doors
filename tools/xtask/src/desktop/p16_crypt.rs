//! P16 CRYPT-A: CryptoAPI keys (advapi32.dll). `cryptprobe.exe guest`
//! (MSVC-built, Windows SDK, no C runtime) compares what it computes in
//! CRYPT_VERIFYCONTEXT contexts (the providers' algorithm lists, session
//! keys imported, derived and generated in every cipher and mode, an RSA
//! key pair from a fixed PRIVATEKEYBLOB with its signatures, decryptions and
//! blobs, HMAC, CBC-MAC and the error codes) with what Windows 11 printed
//! (`cryptprobe.expected.h`, from `cryptprobe.exe raw` on the owner's PC,
//! which saves no key), then keeps a signature key in a key container
//! across contexts and deletes it, which only the guest may do.
use super::p15_connect::{Probe, build, run_probe, stage};
use super::*;

const CRYPTPROBE: Probe = Probe {
    name: "cryptprobe",
    directory: "tests/fixtures/windows-security",
    libraries: &["kernel32.lib", "AdvAPI32.Lib"],
    modules: &["advapi32.dll", "kernel32.dll"],
};
const RESOLUTION: &str = "1280x720";
/// Steps cryptprobe reports, in order.
const STEPS: [&str; 9] = [
    "providers",
    "import",
    "modes",
    "derive",
    "rsa",
    "hmac",
    "extras",
    "genkey",
    "container",
];

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/crypt-a", run.id))?;
    let fixture = build(run, &target, &CRYPTPROBE)?;
    let staged = stage(
        run,
        image,
        std::slice::from_ref(&fixture),
        &target,
        "desktop-crypt",
        &["windows/advapi32.dll"],
    )?;
    let disk = super::win32::fresh_disk(run, "crypt-a")?;
    let mut record = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(3));
        super::win32::open_terminal(run, &record, &mut c)?;
        thread::sleep(Duration::from_secs(2));
        let probe = run_probe(
            run,
            &record,
            &mut c,
            &fixture,
            "cryptprobe.exe guest",
            "CRYPT-A",
            &STEPS,
        )?;
        let frame = scene::capture_settled(run, &mut c, "crypt-a-terminal", &[])?;
        let shot = super::win32::keep(run, &frame, "crypt-a-terminal")?;
        Ok(json!({"probe":probe,"screenshot":shot}))
    })();
    let details = super::win32::quit_on_error(run, &record, result, "crypt-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p16-crypt-a", json!({"fixture":fixture,"image":staged,"run":details,"session":record,
        "reference":"Windows 11 26H2 (owner's PC): cryptprobe.exe check 8/8 (CRYPT_VERIFYCONTEXT only: no key is saved; the key container step is guest-only)",
        "scope":"CRYPT-A: an MSVC-built probe using CryptoAPI keys on the Base, Enhanced, Strong and Enhanced RSA and AES providers: PP_ENUMALGS(_EX), PLAINTEXTKEYBLOB import of RC2, RC4, DES, triple DES and AES keys with their parameters and ciphertexts, ECB/CBC/CFB modes, pieces and padding errors, CryptDeriveKey from MD5, SHA-1 and SHA-256, a fixed RSA key's PKCS #1 signatures, decryptions (v1.5 and OAEP), SIMPLEBLOB and public/private blobs, HMAC with five hashes, CBC-MAC, SSL3 SHAMD5, session-key hashing, CryptGenKey sizes and error codes, then a signature key kept in a key container"}));
    Ok(())
}
