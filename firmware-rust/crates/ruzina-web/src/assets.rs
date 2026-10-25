//! The Web Remote page, built into the image (M2.5 serves `index.html`,
//! `style.css` and `app.js` the same way). It uses only the routes this
//! crate implements: `/api/status`, `/api/control`, `/api/load` and the paged
//! `/api/library`.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Asset {
    pub content_type: &'static str,
    pub body: &'static [u8],
}

pub const INDEX: Asset = Asset {
    content_type: "text/html; charset=utf-8",
    body: include_bytes!("../web/index.html"),
};
pub const STYLE: Asset = Asset {
    content_type: "text/css; charset=utf-8",
    body: include_bytes!("../web/style.css"),
};
pub const APP: Asset = Asset {
    content_type: "text/javascript; charset=utf-8",
    body: include_bytes!("../web/app.js"),
};

/// The largest asset; an adapter's response buffer must hold its head too.
pub const LARGEST_BYTES: usize = max(INDEX.body.len(), max(STYLE.body.len(), APP.body.len()));

const fn max(a: usize, b: usize) -> usize {
    if a > b { a } else { b }
}
