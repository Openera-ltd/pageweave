//! Translation with the installed model (en → de).

use pw_translate::{default_resource_dir, Translator};

#[test]
fn english_to_german() {
    let dir = default_resource_dir().expect("translation models installed");
    let mut t = Translator::load(&dir, "en", "de").expect("load");
    let started = std::time::Instant::now();
    let a = t.sentence("The house is small.").expect("translate");
    let b = t.text("Good morning. How are you?\n\nThe river flows into the sea.", &|| false, &|_, _| {}).expect("text");
    eprintln!("{a:?} {b:?} in {:?}", started.elapsed());
    assert_eq!(a, "Das Haus ist klein.");
    assert!(b.contains("\n\n"), "{b}");
}
