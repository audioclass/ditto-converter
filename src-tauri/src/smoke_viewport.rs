//! Smoke-only conversion: unobscured viewport points to NSWindow content points.
pub fn content_size(viewport: (f64, f64), content: (f64, f64), layout: (f64, f64)) -> (f64, f64) {
    (
        viewport.0 + content.0 - layout.0,
        viewport.1 + content.1 - layout.1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measured_insets_are_not_a_titlebar_constant() {
        for inset in [0., 17., 31.5, 45.] {
            for viewport in [
                (780., 420.),
                (856., 420.),
                (780., 480.),
                (872., 480.),
                (780., 600.),
                (1080., 760.),
            ] {
                let request = content_size(viewport, (1000., 700.), (1000., 700. - inset));
                assert_eq!((request.0, request.1 - inset), viewport);
            }
        }
        assert_eq!(
            content_size((780., 420.), (992., 649.), (982., 621.)),
            (790., 448.)
        );
    }

    #[test]
    fn captured_fullsize_content_view_targets_unobscured_client() {
        let requested = content_size((780., 420.), (992., 649.), (992., 621.));
        // Replay the captured native-to-DOM difference, not a native Mac pass.
        assert_eq!((requested.0, requested.1 - (649. - 621.)), (780., 420.));
    }
}
