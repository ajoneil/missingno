//! The VDP's memory decoded into graphics surfaces, gated on a consumer
//! having asked for them.

use missingno_core::graphics::GraphicsView;

use crate::console::Sg1000;

/// The graphics surfaces the VDP's memory decodes to, or `None` when no
/// consumer asked for them.
pub fn graphics_view(sg: &Sg1000) -> Option<GraphicsView> {
    sg.graphics_capture()
        .then(|| missingno_ti_vdp::graphics::graphics_view(sg.vdp()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use missingno_ti_vdp::Standard;

    #[test]
    fn capture_off_decodes_nothing() {
        let mut console =
            Sg1000::new(&[0u8; 0x2000], None, Standard::Ntsc).expect("flat cartridge image");
        assert!(graphics_view(&console).is_none());
        console.set_graphics_capture(true);
        let view = graphics_view(&console).expect("surfaces decoded");
        assert_eq!(view.atlases.len(), 2);
        assert_eq!(view.maps.len(), 1);
        assert!(view.objects.is_some());
    }
}
