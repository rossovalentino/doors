//! Deterministic composition corpus: each case types a key script into one
//! engine and checks the text the application ends with (commits plus
//! forwarded keys). Dictionaries are tiny synthetic fixtures built here.
use door_abi::graphics as g;
use door_ime::dict::Dictionary;
use door_ime::engine::{Engine, KeyIn};
use door_ime::{hangul::Hangul, japanese::Japanese, pinyin::Pinyin};
use door_keyboard_layouts::vk;
fn fixture(entries: &[&str]) -> Dictionary {
    let mut lines: Vec<&str> = entries.to_vec();
    lines.sort_by(|a, b| a.split('\t').next().cmp(&b.split('\t').next()));
    let mut text = String::from("# door-ime dictionary v1\n# synthetic test fixture\n");
    for l in lines {
        text.push_str(l);
        text.push('\n');
    }
    Dictionary::parse(Box::leak(text.into_bytes().into_boxed_slice())).unwrap()
}
fn special(name: &str) -> Vec<KeyIn> {
    let s = g::SHIFT | g::LEFT_SHIFT;
    let k = |v: u8, f: u32| KeyIn::special(v, f);
    match name {
        "space" => vec![KeyIn::typed(' ')],
        "enter" => vec![k(vk::RETURN, 0)],
        "bs" => vec![k(vk::BACK, 0)],
        "esc" => vec![k(vk::ESCAPE, 0)],
        "left" => vec![k(vk::LEFT, 0)],
        "right" => vec![k(vk::RIGHT, 0)],
        "sleft" => vec![k(vk::LEFT, s)],
        "sright" => vec![k(vk::RIGHT, s)],
        "down" => vec![k(vk::DOWN, 0)],
        "up" => vec![k(vk::UP, 0)],
        "hangul" => vec![k(vk::HANGUL, 0)],
        "kanji" => vec![k(vk::KANJI, 0)],
        "f7" => vec![k(vk::F7, 0)],
        "f10" => vec![k(vk::F10, 0)],
        "shift" => vec![
            KeyIn {
                code: 0x2a,
                vk: vk::SHIFT,
                flags: g::DOWN | s,
                ch: None,
            },
            KeyIn {
                code: 0x2a,
                vk: vk::SHIFT,
                flags: 0,
                ch: None,
            },
        ],
        _ => panic!("unknown key {name}"),
    }
}
/// Run a script: plain characters are typed, `{name}` are special keys.
fn run(engine: &mut dyn Engine, script: &str) -> String {
    let mut out = String::new();
    let mut keys = Vec::new();
    let mut rest = script;
    while let Some(c) = rest.chars().next() {
        if c == '{' {
            let end = rest.find('}').unwrap();
            keys.extend(special(&rest[1..end]));
            rest = &rest[end + 1..];
        } else {
            keys.push(KeyIn::typed(c));
            rest = &rest[c.len_utf8()..];
        }
    }
    for k in keys {
        let o = engine.key(&k);
        out.push_str(&o.commit);
        if !o.handled && k.down() {
            // The application inserts a forwarded key's text.
            if let Some(c) = k.ch.filter(|c| !c.is_control()) {
                out.push(c);
            }
        }
    }
    out.push_str(&engine.complete());
    out
}
#[test]
fn korean_two_set_corpus() {
    for (script, expected) in [
        ("gksrmf", "한글"),
        ("dkssudgktpdy", "안녕하세요"),
        ("rkqt", "값"),
        ("ekfrdl", "닭이"),
        ("ekfrk", "달가"),
        ("ehoqh", "돼보"),
        ("gks{bs}", "하"),
        ("gk{space}rk", "하 가"),
        ("gk{hangul}ab{hangul}rk", "하ab가"),
        ("Rk", "까"),
        ("dhk", "와"),
        ("k", "ㅏ"),
    ] {
        assert_eq!(run(&mut Hangul::new(), script), expected, "{script}");
    }
}
fn pinyin() -> Pinyin {
    Pinyin::new(fixture(&[
        "an\t安",
        "guo\t国\t过",
        "hao\t好\t号",
        "ni\t你\t呢\t尼",
        "ni'hao\t你好",
        "xi\t西",
        "xi'an\t西安",
        "xian\t先\t现",
        "zhong\t中",
        "zhong'guo\t中国",
    ]))
}
#[test]
fn pinyin_corpus() {
    for (script, expected) in [
        ("nihao{space}", "你好"),
        ("nihao2{space}", "你好"),
        ("nihao32", "呢号"),
        ("zhongguo{space}", "中国"),
        ("xian{space}", "先"),
        ("xi'an{space}", "西安"),
        ("nih{space}", "你好"),
        ("ni{enter}", "ni"),
        ("ni{esc}", ""),
        ("nihao,", "你好，"),
        ("{shift}ni", "ni"),
        ("ni3ni{space}", "尼尼"),
        ("nihao{right}{space}{enter}", "你hao"),
        ("nihao{bs}{bs}{bs}{space}", "你"),
        ("a.", "a。"),
    ] {
        assert_eq!(run(&mut pinyin(), script), expected, "{script}");
    }
}
fn japanese() -> Japanese {
    Japanese::new(fixture(&[
        "か\t蚊,7000",
        "かく\t書く,4000,v5k\t描く,5000,v5k",
        "がっこう\t学校,3000",
        "ご\t語,6000\t後,6500",
        "です\tです,1000,p",
        "に\tに,1000,p",
        "にほん\t日本,3000",
        "にほんご\t日本語,2500",
        "は\tは,1000,p",
        "わたし\t私,3000",
    ]))
}
#[test]
fn japanese_corpus() {
    for (script, expected) in [
        (
            "watashihagakkounikakimasu{space}{enter}",
            "私は学校に書きます",
        ),
        ("nihongo{space}{enter}", "日本語"),
        ("nihongo{enter}", "にほんご"),
        ("kakimasu{space}{space}{enter}", "描きます"),
        ("nihongo{f7}{enter}", "ニホンゴ"),
        ("nihongo{f10}{enter}", "nihongo"),
        ("nihongo{space}{sleft}{right}{space}{enter}", "日本後"),
        ("{kanji}abc", "abc"),
        ("ra-men{enter}", "らーめん"),
        ("ni{bs}{bs}a{enter}", "あ"),
        ("watashi{space}{esc}{enter}", "わたし"),
    ] {
        assert_eq!(run(&mut japanese(), script), expected, "{script}");
    }
}
/// TEXT-01's key scripts against the shipped dictionaries (`assets/ime`,
/// present in owner builds; skipped otherwise): what the guest suite types
/// must commit exactly what its disk audit expects.
#[test]
fn text01_scripts_with_the_shipped_dictionaries() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/ime");
    let load = |name: &str| {
        let bytes = std::fs::read(root.join(name)).ok()?;
        Some(Dictionary::parse(Box::leak(bytes.into_boxed_slice())).unwrap())
    };
    let (Some(pinyin), Some(japanese)) = (load("pinyin.dict"), load("japanese.dict")) else {
        eprintln!("assets/ime dictionaries absent: TEXT-01 scripts not checked");
        return;
    };
    assert_eq!(run(&mut Pinyin::new(pinyin), "nihao{space}"), "你好");
    let mut ja = Japanese::new(japanese);
    // Space converts, Space opens the list on the second candidate, 1
    // picks the first, Enter commits.
    assert_eq!(run(&mut ja, "nihongo{space}{space}1{enter}"), "日本語");
    // Zero-cost kana morphemes no longer fragment readings.
    assert_eq!(run(&mut ja, "nihongo{space}{enter}"), "日本語");
    assert_eq!(
        run(&mut ja, "watashihagakkouniikimasu{space}{enter}"),
        "私は学校に行きます"
    );
    // Up wraps to the last candidate: the ZWJ family emoji.
    assert_eq!(
        run(&mut ja, "kazoku{space}{up}{enter}"),
        "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}"
    );
    assert_eq!(run(&mut Hangul::new(), "gksrnrdj "), "한국어 ");
}
