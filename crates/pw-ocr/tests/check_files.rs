//! Resource audit: a copy of the installed OCR folder with one file altered and one removed.

#[test]
fn altered_and_missing_files_are_reported() {
    let Some(dir) = pw_ocr::default_resource_dir() else {
        assert!(std::env::var_os("CI").is_none(), "OCR resources missing in CI");
        eprintln!("OCR resources not installed; skipping");
        return;
    };
    let ok = pw_ocr::check_files(&dir).expect("check");
    assert!(ok.iter().all(|f| f.matches == Some(true)), "{ok:?}");
    let tmp = tempfile::tempdir().expect("tmp");
    for f in &ok {
        std::fs::copy(dir.join(&f.name), tmp.path().join(&f.name)).expect("copy");
    }
    let mut dict = std::fs::read(tmp.path().join(pw_ocr::DICT_FILE)).expect("read");
    dict.push(b'x');
    std::fs::write(tmp.path().join(pw_ocr::DICT_FILE), dict).expect("write");
    std::fs::remove_file(tmp.path().join(pw_ocr::DET_FILE)).expect("rm");
    let got = pw_ocr::check_files(tmp.path()).expect("check");
    let by = |n: &str| got.iter().find(|f| f.name == n).expect(n).matches;
    assert_eq!(by(pw_ocr::DICT_FILE), Some(false), "altered");
    assert_eq!(by(pw_ocr::DET_FILE), None, "missing");
    assert_eq!(by(pw_ocr::REC_FILE), Some(true));
    assert_eq!(by(pw_ocr::RUNTIME_FILE), Some(true), "runtime library matches its pinned hash");
}
