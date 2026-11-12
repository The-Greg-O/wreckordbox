//! The layout's naming rules, case by case from its own tables.
#![allow(
    clippy::pedantic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic
)]

use wreckordbox_native_stems::naming::{folder_name, suffixed_folder_name, track_key, Unnameable};

#[test]
fn the_key_is_the_file_name_up_to_its_last_dot() {
    // "The track key", the layout's table.
    for (path, key) in [
        ("Contents/Artist/Artist - Song.mp3", "Artist - Song"),
        ("Music/Song.v2.flac", "Song.v2"),
        ("Music/Intro .mp3", "Intro "),
        ("Music/README", "README"),
    ] {
        assert_eq!(track_key(path), key, "{path}");
    }
}

#[test]
fn a_key_the_stick_can_store_is_the_folder_name_as_written() {
    for key in [
        "Artist - Song",
        "Song.v2",
        "Ünïcödé — 曲",
        // Not device names: only COM1 to COM9 and LPT1 to LPT9 are.
        "CONSOLE",
        "COM0",
        "COM10",
        "LPT0",
        "Song (CON)",
        "Song_",
    ] {
        assert_eq!(folder_name(key).unwrap(), key, "{key}");
    }
}

#[test]
fn a_key_fat_cannot_store_as_written_gets_an_underscore() {
    // "When the AZ-MODS app appends `_`", the layout's table and rules.
    for (key, folder) in [
        ("Song.", "Song._"),
        ("Intro ", "Intro _"),
        ("CON", "CON_"),
        ("con.remix", "con.remix_"),
        ("PRN", "PRN_"),
        ("Aux", "Aux_"),
        ("nul", "nul_"),
        ("COM1", "COM1_"),
        ("com9", "com9_"),
        ("LPT1", "LPT1_"),
        ("lpt9.live", "lpt9.live_"),
        // "the part of the key before its first `.`, with trailing spaces
        // removed, is a Windows device name"
        ("AUX  .set", "AUX  .set_"),
    ] {
        assert_eq!(folder_name(key).unwrap(), folder, "{key:?}");
    }
}

#[test]
fn a_key_no_folder_can_carry_is_refused() {
    // "Keys that cannot have stems".
    for key in ["", " ", "   "] {
        assert_eq!(folder_name(key), Err(Unnameable::Empty), "{key:?}");
    }
    for key in [".hidden", ".", "..x"] {
        assert_eq!(folder_name(key), Err(Unnameable::StartsWithDot), "{key:?}");
    }
    for character in [
        '\\', '/', ':', '*', '?', '"', '<', '>', '|', '\u{1}', '\t', '\u{1f}',
    ] {
        let key = format!("So{character}ng");
        assert_eq!(
            folder_name(&key),
            Err(Unnameable::Character(character)),
            "{key:?}"
        );
    }
}

#[test]
fn a_refusal_says_why_in_words() {
    assert!(folder_name("").unwrap_err().to_string().contains("empty"));
    assert!(folder_name(".x").unwrap_err().to_string().contains("dot"));
    assert!(folder_name("a:b").unwrap_err().to_string().contains("':'"));
    assert!(folder_name("a\tb")
        .unwrap_err()
        .to_string()
        .contains("U+0009"));
}

#[test]
fn a_second_song_of_the_same_name_gets_the_crc_32_of_its_path_on_the_stick() {
    // "The 8-hex suffix": CRC-32 of the USB-relative path (UTF-8, forward
    // slashes), 8 lowercase hex digits. Values from Python's zlib.crc32.
    assert_eq!(
        suffixed_folder_name("Artist - Song", "Other/Artist - Song.mp3"),
        "Artist - Song-ce7c3905"
    );
    assert_eq!(suffixed_folder_name("x", "123456789"), "x-cbf43926");
}
