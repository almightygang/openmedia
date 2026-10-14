//! AniList integration — port of `main/ipc/anilist/`.
//!
//! Public GraphQL helper, authed helper with viewer-id cache, OAuth
//! (system-browser + `openmedia://` deep-link callback), media page /
//! search / details / popular-season, list + favourites mutations, and
//! MAL→AniList id resolution for providers.

pub mod api;
pub mod favourites;
pub mod list;
pub mod media;
pub mod oauth;
pub mod state;

pub use state::AniListState;
