//! P16 TRUST-A: WinVerifyTrust and the catalog store (wintrust.dll).
//! `trustprobe.exe` (MSVC-built, Windows SDK, no C runtime) first compares
//! what it reads with what Windows 11 printed (`trustprobe.expected.h`, from
//! `trustprobe.exe raw` on the owner's PC, cache-only and read-only): its
//! fixtures' signatures under a root no store trusts, catalog hashes and
//! catalog member checks. Then, in an elevated Doorsh, it does what only the
//! guest may: trusts the test root in the machine Root store, checks
//! revocation against the CRL this host serves at 10.0.2.2:28480 (the URL
//! the fixtures' certificates name), adds the catalog to
//! C:\Windows\System32\CatRoot, finds it by hash, removes it and untrusts the
//! root.
use super::p15_connect::{Probe, build, run_probe, stage_with};
use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

const TRUSTPROBE: Probe = Probe {
    name: "trustprobe",
    directory: "tests/fixtures/windows-security",
    libraries: &["kernel32.lib", "AdvAPI32.Lib", "WinTrust.Lib"],
    modules: &["advapi32.dll", "kernel32.dll", "wintrust.dll"],
};
const RESOLUTION: &str = "1280x720";
/// Steps trustprobe reports in the guest, in order.
const STEPS: [&str; 7] = [
    "files",
    "hashes",
    "catalog",
    "trusted",
    "revocation",
    "store",
    "untrust",
];
/// The fixtures' CRL distribution point: http://10.0.2.2:28480/<CRL_PATH>.
const CRL_PORT: u16 = 28480;
const CRL_PATH: &str = "/doors-trust-test.crl";
const FIXTURES: &str = "crates/authenticode/tests/data";

/// Serves the fixtures' CRL on 127.0.0.1:28480 (10.0.2.2 to the guest)
/// until `stop`; records each request line and User-Agent.
fn crl_host(
    crl: Vec<u8>,
    stop: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
) -> Result<thread::JoinHandle<()>, String> {
    let listener = TcpListener::bind(("127.0.0.1", CRL_PORT))
        .map_err(|e| format!("the CRL host needs 127.0.0.1:{CRL_PORT}: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    Ok(thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            let Ok((mut stream, _)) = listener.accept() else {
                thread::sleep(Duration::from_millis(50));
                continue;
            };
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut request = Vec::new();
            let mut buffer = [0u8; 2048];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 16 * 1024 {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => request.extend_from_slice(&buffer[..n]),
                }
            }
            let text = String::from_utf8_lossy(&request).into_owned();
            let first = text.lines().next().unwrap_or("").to_owned();
            let agent = text
                .lines()
                .find(|l| l.to_ascii_lowercase().starts_with("user-agent:"))
                .unwrap_or("")
                .to_owned();
            requests
                .lock()
                .map(|mut r| r.push(format!("{first} | {agent}")))
                .ok();
            let response = if first.starts_with(&format!("GET {CRL_PATH} ")) {
                let mut r = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/pkix-crl\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    crl.len()
                )
                .into_bytes();
                r.extend_from_slice(&crl);
                r
            } else {
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
            };
            let _ = stream.write_all(&response);
        }
    }))
}

pub(super) fn verify(run: &mut Run, image: &str) -> Result<(), String> {
    let target = run.generated_directory(&format!("out/builds/{}/trust-a", run.id))?;
    let fixture = build(run, &target, &TRUSTPROBE)?;
    let read = |path: &str| -> Result<Vec<u8>, String> {
        fs::read(run.root.join(FIXTURES).join(path)).map_err(|e| format!("{path}: {e}"))
    };
    let data = vec![
        ("signed-good.exe", read("trust/signed-good.exe")?),
        ("signed-revoked.exe", read("trust/signed-revoked.exe")?),
        ("hello.exe", read("unsigned.exe")?),
        ("readme.txt", read("trust/readme.txt")?),
        ("doors-trust-test.cat", read("trust/doors-trust-test.cat")?),
        ("root.der", read("trust/root.der")?),
    ];
    let crl = read("trust/doors-trust-test.crl")?;
    let staged = stage_with(
        run,
        image,
        std::slice::from_ref(&fixture),
        &data,
        &target,
        "desktop-trust",
        &[
            "windows/wintrust.dll",
            "windows/winhttp.dll",
            "windows/advapi32.dll",
            "windows/kernel32.dll",
            "windows/ntdll.dll",
        ],
    )?;
    let stop = Arc::new(AtomicBool::new(false));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let host = crl_host(crl, stop.clone(), requests.clone())?;
    let disk = super::win32::fresh_disk(run, "trust-a")?;
    let mut record = session::launch_desktop_accelerated(run, &staged, &disk, false, RESOLUTION)?;
    let result: Result<Value, String> = (|| {
        let mut c = connection(&record)?;
        thread::sleep(Duration::from_secs(3));
        // Win+R doorsh, Ctrl+Shift+Enter: User Account Control, then the
        // elevated Terminal on a Doorsh tab (the Root store and the
        // catalog store are the machine's).
        let before = serial(&run.root, &record)?.len();
        key(&mut c, &["meta_l", "r"])?;
        thread::sleep(Duration::from_millis(400));
        for ch in "doorsh".chars() {
            super::administration::type_character(&mut c, ch)?;
        }
        key(&mut c, &["ctrl", "shift", "ret"])?;
        super::win32::wait_after(run, &record, before, "DOOR_P16 RUN elevated", 40)?;
        super::win32::wait_after(run, &record, before, "DOOR_P16 UAC PROMPT", 40)?;
        key(&mut c, &["alt", "y"])?;
        super::win32::wait_after(
            run,
            &record,
            before,
            "DOOR_P16 ELEVATED TERMINAL launched",
            40,
        )?;
        thread::sleep(Duration::from_secs(3));
        let probe = run_probe(
            run,
            &record,
            &mut c,
            &fixture,
            "trustprobe.exe guest",
            "TRUST-A",
            &STEPS,
        )?;
        let fetched: Vec<String> = requests.lock().map(|r| r.clone()).unwrap_or_default();
        // The CRL was fetched once: the later checks used the user's
        // CryptnetUrlCache.
        if fetched.len() != 1 || !fetched[0].starts_with(&format!("GET {CRL_PATH} ")) {
            return Err(format!("CRL requests: {fetched:?}"));
        }
        let frame = scene::capture_settled(run, &mut c, "trust-a-terminal", &[])?;
        let shot = super::win32::keep(run, &frame, "trust-a-terminal")?;
        Ok(json!({"probe":probe,"crl_requests":fetched,"screenshot":shot}))
    })();
    stop.store(true, Ordering::Relaxed);
    let _ = host.join();
    let details = super::win32::quit_on_error(run, &record, result, "trust-a-failure")?;
    let mut c = connection(&record)?;
    super::win32::finish(run, &mut record, &mut c)?;
    run.check("p16-trust-a", json!({"fixture":fixture,"image":staged,"run":details,"session":record,
        "reference":"Windows 11 26H2 (owner's PC): trustprobe.exe check 3/3 (cache-only URL retrieval, nothing trusted or added; raw output in compat/dlls/wintrust/tests/reference/trustprobe-raw.txt, including a System32 file verified through its CatRoot catalog)",
        "scope":"TRUST-A: an MSVC-built probe using wintrust.dll: WinVerifyTrust (GENERIC_VERIFY_V2) on programs signed by a test publisher and a revoked one, an unsigned program, a text file and a missing file (CERT_E_CHAINING, TRUST_E_NOSIGNATURE, TRUST_E_SUBJECT_FORM_UNKNOWN, CRYPT_E_FILE_ERROR as Windows answers), CryptCATAdmin SHA-1 and SHA-256 hashes, catalog member checks in six ways of passing a member; then, elevated, the test root in the machine Root store, CRL revocation (fetched once over WinHTTP from the host, then from the CryptnetUrlCache, CERT_E_REVOKED and CERT_E_REVOCATION_FAILURE), CryptCATAdminAddCatalog into CatRoot\\{F750E6C3-38EE-11D1-85E5-00C04FC295EE}, EnumCatalogFromHash, CatalogInfoFromContext, a catalog-signed file verified, RemoveCatalog and the root removed again"}));
    Ok(())
}
