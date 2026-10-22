//! Read-only calls in reader processes (D-018): same answers as in-process calls, crashes and
//! hangs fail only their call, a reader that died while idle is replaced silently.

use pw_pdf::service::{run_call, ReadCall, ReadReply, ReaderPool};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pageweave"))
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/pdf").join(name)
}

#[test]
fn answers_match_the_engine_called_directly() {
    let pool = ReaderPool::new(exe(), 2);
    let path = fixture("article.pdf");
    let calls = [
        ReadCall::Open { path: path.clone() },
        ReadCall::Render { path: path.clone(), page: 0, scale: 0.5 },
        ReadCall::PageText { path: path.clone(), page: 1 },
        ReadCall::Search { path: path.clone(), query: "river".into(), match_case: false, whole_word: false },
        ReadCall::ListAnnotations { path: path.clone() },
        ReadCall::PageObjects { path: path.clone(), page: 0 },
        ReadCall::FormFields { path: fixture("form-fields.pdf") },
        ReadCall::SanitizeInspect { path: fixture("hidden-info.pdf") },
    ];
    for c in &calls {
        let a = pool.call(c, None).unwrap_or_else(|e| panic!("{c:?}: {e:?}"));
        let b = run_call(c, None).expect("in-process");
        assert_eq!(a, b, "{c:?}");
    }
    // Passwords travel with the call.
    let enc = ReadCall::Open { path: fixture("encrypted-rc4-user-password.pdf") };
    assert_eq!(pool.call(&enc, None).expect_err("needs password").category, pw_core::ErrorCategory::PasswordRequired);
    assert!(pool.call(&enc, Some("pw-user-test")).is_ok());
    // Parallel calls from many threads all get their own answers.
    std::thread::scope(|s| {
        for p in 0..5u32 {
            let pool = &pool;
            s.spawn(move || {
                let r =
                    pool.call(&ReadCall::PageText { path: fixture("five-pages.pdf"), page: p }, None).expect("text");
                let ReadReply::Json(v) = r else { panic!("json") };
                assert!(v.to_string().contains(&format!("PWMARK00{}", p + 1)), "page {p}: {v}");
            });
        }
    });
}

#[test]
fn a_crash_fails_only_its_call_and_is_not_retried() {
    // Debug builds honour the test hook: rendering page 2 aborts the reader.
    let pool = ReaderPool::new(exe(), 1).with_env("PW_READER_TEST", "abort:2");
    let path = fixture("five-pages.pdf");
    let render = |page| pool.call(&ReadCall::Render { path: path.clone(), page, scale: 0.3 }, None);
    assert!(matches!(render(0), Ok(ReadReply::Bytes(_))));
    let before = pool.reader_pids();
    let e = render(2).expect_err("crashes");
    assert_eq!(e.code, "engine.crashed");
    // The next call gets a new reader and works.
    assert!(matches!(render(1), Ok(ReadReply::Bytes(_))));
    assert_ne!(pool.reader_pids(), before, "replaced");
    // The crashing call is answered at once, without crashing another reader.
    let pids = pool.reader_pids();
    assert_eq!(render(2).expect_err("remembered").code, "engine.crashed");
    assert_eq!(pool.reader_pids(), pids, "no new crash");
    pool.forget_crashes_for(&path);
    assert_eq!(render(2).expect_err("crashes again once forgotten").code, "engine.crashed");
}

#[test]
fn a_hang_times_out_and_an_idle_reader_that_died_is_replaced() {
    let pool = ReaderPool::with_timeout(exe(), 1, Duration::from_secs(3)).with_env("PW_READER_TEST", "hang:3");
    let path = fixture("five-pages.pdf");
    let t = std::time::Instant::now();
    let e = pool.call(&ReadCall::Render { path: path.clone(), page: 3, scale: 0.3 }, None).expect_err("hangs");
    assert_eq!(e.code, "engine.timeout");
    assert!(t.elapsed() < Duration::from_secs(10));
    // Kill the idle reader from outside: the next call is not blamed.
    assert!(pool.call(&ReadCall::Open { path: path.clone() }, None).is_ok());
    for pid in pool.reader_pids() {
        let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
    std::thread::sleep(Duration::from_millis(200));
    assert!(pool.call(&ReadCall::Open { path }, None).is_ok(), "replaced silently");
}

#[test]
fn page_images_through_a_reader_cost_little_more_than_in_process() {
    let path = fixture("photo-heavy.pdf");
    let pool = ReaderPool::new(exe(), 1);
    let call = ReadCall::Render { path: path.clone(), page: 0, scale: 1.5 };
    pool.call(&call, None).expect("warm up");
    run_call(&call, None).expect("warm up");
    let n = 10;
    let t = std::time::Instant::now();
    for _ in 0..n {
        pool.call(&call, None).expect("reader");
    }
    let reader = t.elapsed() / n;
    let t = std::time::Instant::now();
    for _ in 0..n {
        run_call(&call, None).expect("direct");
    }
    let direct = t.elapsed() / n;
    eprintln!("page 1 of photo-heavy.pdf at 150%: reader {reader:?}, in process {direct:?}");
    assert!(reader < direct * 2 + Duration::from_millis(50), "reader {reader:?} vs {direct:?}");
}
