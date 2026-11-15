use super::*;

fn handle(request: &str) -> std::string::String {
    let mut out = std::vec![0u8; ruzina_web::assets::LARGEST_BYTES + 1024];
    // SAFETY: both buffers are valid for their lengths.
    let len = unsafe {
        ruzina_web_handle(
            7,
            request.as_ptr(),
            request.len(),
            out.as_mut_ptr(),
            out.len(),
        )
    };
    std::string::String::from_utf8_lossy(&out[..len]).into_owned()
}

fn drain_web() -> std::vec::Vec<(usize, TouchControl)> {
    core::iter::from_fn(|| deck_view::WEB.pop()).collect()
}

#[test]
fn play_and_cue_take_the_touch_path_and_other_controls_are_refused() {
    let _decks = crate::DECK_TESTS.lock().unwrap_or_else(|e| e.into_inner());
    drain_web();
    let post = |query: &str| {
        handle(&std::format!(
            "POST /api/control?{query} HTTP/1.1\r\nHost: 192.168.4.1\r\nOrigin: http://192.168.4.1\r\nX-DDJ-Control: 1\r\nContent-Length: 0\r\n\r\n"
        ))
    };
    let play = post("action=play_pause&deck=2");
    assert!(play.starts_with("HTTP/1.1 200"), "{play}");
    assert_eq!(drain_web(), [(1, TouchControl::Play)], "the press only");
    let cue = post("action=cue&deck=1");
    assert!(cue.starts_with("HTTP/1.1 200"), "{cue}");
    assert_eq!(
        drain_web(),
        [(0, TouchControl::Cue(true)), (0, TouchControl::Cue(false))]
    );
    let volume = post("action=volume&deck=1&value=16383");
    assert!(volume.starts_with("HTTP/1.1 503"), "{volume}");
    assert_eq!(drain_web(), []);
}

#[test]
fn the_page_and_the_status_answer_for_the_access_point_only() {
    let page = handle("GET / HTTP/1.1\r\nHost: 192.168.4.1\r\n\r\n");
    assert!(
        page.starts_with("HTTP/1.1 200"),
        "{}",
        &page[..page.len().min(80)]
    );
    let status = handle("GET /api/status HTTP/1.1\r\nHost: 192.168.4.1\r\n\r\n");
    assert!(
        status.starts_with("HTTP/1.1 200"),
        "{}",
        &status[..status.len().min(80)]
    );
    assert!(
        status.contains("\"deck1\"") && status.contains("\"deck2\""),
        "{status}"
    );
    let foreign = handle("GET /api/status HTTP/1.1\r\nHost: example.com\r\n\r\n");
    assert!(!foreign.starts_with("HTTP/1.1 200"), "{foreign}");
}
