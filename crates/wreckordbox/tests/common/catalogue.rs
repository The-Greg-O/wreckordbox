//! A fixture Library with enough in it to tell every track filter apart.

use rbl_db::fixture::{self, Shape};

use super::{Details, Sandbox};

/// The Library, and the identifiers of what was added to the fixture.
pub struct Catalogue {
    pub sandbox: Sandbox,
    /// The imported track: added today, 95 seconds long, not analysed.
    pub fresh: String,
    /// The playlist "Warm up set": tracks 10012 then 10003.
    pub warm_up_set: String,
}

/// Sixty fixture tracks (10000 to 10059, 128.00 to 128.59 BPM, 5:00 long,
/// analysed, in no playlist past 10014), of which three are described:
///
/// | id    | title          | artist       | key | Energy | genre      | year | colour | MyTags       |
/// |-------|----------------|--------------|-----|--------|------------|------|--------|--------------|
/// | 10003 | Midnight Drive | Ada Lovelace | 8A  | 4      | Tech House | 2021 | Green  | Peak         |
/// | 10007 | Sunrise        | Ada Lovelace | 11B | -      | Deep House | 2019 | Red    | Warm-up      |
/// | 10012 | Midnight Oil   | Grace Hopper | 8A  | 5      | Techno     | 2023 | -      | Peak, Vocal  |
///
/// plus one imported track, the playlist "Warm up set" (10012, 10003) and
/// the intelligent playlist "House" (genre contains "house"), beside the
/// fixture's own "Playlist 0" (10000 to 10004), "Playlist 1" and
/// "Playlist 2".
pub fn catalogue() -> Catalogue {
    let sandbox = Sandbox::with_shape(Shape {
        tracks: 60,
        ..Shape::default()
    });
    let vocal = sandbox.create_my_tag("Vocal");

    sandbox.describe_track(3, "Midnight Drive", "Ada Lovelace", "8A", 4);
    sandbox.detail_track(
        3,
        &Details {
            album: "Night Moves",
            genre: "Tech House",
            label: "Analytical",
            year: "2021",
            play_count: "7",
            comment: "needs an intro edit",
            colour_id: Some(5),
            my_tags: &[fixture::MY_TAG_PEAK],
        },
    );
    sandbox.describe_track(7, "Sunrise", "Ada Lovelace", "11B", 0);
    sandbox.detail_track(
        7,
        &Details {
            genre: "Deep House",
            year: "2019",
            colour_id: Some(2),
            my_tags: &[fixture::MY_TAG_WARM_UP],
            ..Details::default()
        },
    );
    sandbox.describe_track(12, "Midnight Oil", "Grace Hopper", "8A", 5);
    sandbox.detail_track(
        12,
        &Details {
            album: "Bugs",
            genre: "Techno",
            label: "Compiler Records",
            year: "2023",
            play_count: "2",
            comment: "closer",
            my_tags: &[fixture::MY_TAG_PEAK, &vocal],
            ..Details::default()
        },
    );

    let warm_up_set = sandbox.create_playlist("Warm up set", &[12, 3]);
    sandbox.create_genre_playlist("House", "house");
    let fresh = sandbox.import_track("Fresh Cut.wav", 95);
    Catalogue {
        sandbox,
        fresh,
        warm_up_set,
    }
}
