//! Native startup sizing policy, independent of the windowing runtime.
#[derive(Debug, PartialEq)]
pub struct WindowFit {
    pub inner: (f64, f64),
    pub minimum: (f64, f64),
}

pub fn fit_window(
    preferred: (f64, f64),
    minimum: (f64, f64),
    work: (u32, u32),
    decoration: (u32, u32),
    scale: f64,
) -> WindowFit {
    // Match the existing startup overflow margin: 32 physical pixels per axis.
    // Native backends reapply minimum constraints after overflow fitting, so cap
    // those constraints to the decorated workarea before requesting the size.
    let available = (
        f64::from(
            work.0
                .saturating_sub(decoration.0)
                .saturating_sub(32)
                .max(1),
        ) / scale,
        f64::from(
            work.1
                .saturating_sub(decoration.1)
                .saturating_sub(32)
                .max(1),
        ) / scale,
    );
    WindowFit {
        inner: (preferred.0.min(available.0), preferred.1.min(available.1)),
        minimum: (minimum.0.min(available.0), minimum.1.min(available.1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_high_dpi_workarea_caps_minimum_before_fitting_preferred_size() {
        let fit = fit_window((1080., 760.), (780., 600.), (1744, 960), (26, 71), 2.);
        assert_eq!(fit.minimum, (780., 428.5));
        assert_eq!(fit.inner, (843., 428.5));
    }

    #[test]
    fn ordinary_workarea_keeps_preferred_size_and_ordinary_minimum() {
        let fit = fit_window((1080., 760.), (780., 600.), (2448, 1774), (26, 71), 2.);
        assert_eq!(fit.inner, (1080., 760.));
        assert_eq!(fit.minimum, (780., 600.));
    }

    #[test]
    fn fitting_uses_physical_decorations_and_reserve_at_each_scale() {
        for scale in [1., 1.25, 1.5, 2., 3.] {
            for logical_height in [480., 600., 760.] {
                let work = (1800, (logical_height * scale) as u32);
                let decoration = (16, 71);
                let fit = fit_window((1080., 760.), (780., 600.), work, decoration, scale);
                assert!(fit.inner.0 * scale + 48. <= f64::from(work.0) + 0.001);
                assert!(fit.inner.1 * scale + 103. <= f64::from(work.1) + 0.001);
                assert!(fit.minimum.0 <= fit.inner.0 && fit.minimum.1 <= fit.inner.1);
                assert!(fit.minimum.0 <= 780. && fit.minimum.1 <= 600.);
            }
        }
    }

    #[test]
    fn tiny_workarea_saturates_without_unsigned_underflow() {
        let fit = fit_window((1080., 760.), (780., 600.), (10, 20), (26, 71), 2.);
        assert_eq!(fit.inner, (0.5, 0.5));
        assert_eq!(fit.minimum, fit.inner);
    }
}
