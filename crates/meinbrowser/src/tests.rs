use super::Renderer;

#[test]
fn renderer_draws_a_local_file() {
    let path =
        std::env::temp_dir().join(format!("meinbrowser-{}-anzeige.html", std::process::id()));
    let html = "<!DOCTYPE html><body style='margin: 0; background: #00ff00'>\
                <div style='width: 10px; height: 10px; background: #0000ff'></div>";
    std::fs::write(&path, html).unwrap();
    let renderer = Renderer::start(&path);
    std::fs::remove_file(&path).unwrap();
    let renderer = renderer.unwrap();
    let canvas = renderer.render(40, 30).unwrap();
    assert_eq!((canvas.width(), canvas.height()), (40, 30));
    assert_eq!(canvas.pixel(5, 5), Some(0xff00_00ff));
    assert_eq!(canvas.pixel(20, 20), Some(0xff00_ff00));
    // Die Seite bleibt geladen und passt sich neuen Größen an.
    let canvas = renderer.render(12, 8).unwrap();
    assert_eq!((canvas.width(), canvas.height()), (12, 8));
    assert_eq!(canvas.pixel(11, 7), Some(0xff00_ff00));
}

#[test]
fn missing_files_are_reported() {
    let path = std::env::temp_dir().join(format!("meinbrowser-{}-fehlt.html", std::process::id()));
    let error = Renderer::start(&path).err().unwrap();
    assert!(error.ends_with("lässt sich nicht laden"), "{error}");
}
