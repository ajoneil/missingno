mod emulation;
mod ui;

fn main() {
    // let args: Vec<String> = std::env::args().collect();
    // let filename = &args[1];
    // let path = Path::new(&filename);
    // let mut file = File::open(&path).unwrap();
    // let mut rom = Vec::new();
    // file.read_to_end(&mut rom).unwrap();
    // let mut gb = emulation::gameboy::Gameboy::new(rom);

    ui::run().unwrap();

    // let event_loop = EventLoop::new().unwrap();
    // let window = WindowBuilder::new()
    //     .with_title(&gb.rom_info().title)
    //     .build(&event_loop)
    //     .unwrap();

    // let width = 8 * 16;
    // let height = 8 * (24 + 20);

    // let mut pixels = {
    //     let window_size = window.inner_size();
    //     let surface_texture = SurfaceTexture::new(window_size.width, window_size.height, &window);
    //     Pixels::new(width, height, surface_texture).unwrap()
    // };

    // event_loop.set_control_flow(ControlFlow::Poll);
    // event_loop
    //     .run(move |event, window_target| {
    //         gb.step();

    //         if gb.video().frame_ready() {
    //             window.request_redraw();
    //             gb.take_frame()
    //         }

    //         match event {
    //             Event::WindowEvent {
    //                 event: WindowEvent::CloseRequested,
    //                 ..
    //             } => {
    //                 println!("The close button was pressed; stopping");
    //                 window_target.exit();
    //             }
    //             Event::WindowEvent {
    //                 event: WindowEvent::RedrawRequested,
    //                 ..
    //             } => {
    //                 let framebuffer = pixels.frame_mut();
    //                 const TILES_PER_LINE: usize = 16;
    //                 const PIXEL_BYTES: usize = 4;
    //                 const TILE_PIXELS: usize = 8;
    //                 const FB_SINGLE_TILE_LINE_SIZE: usize = TILE_PIXELS * PIXEL_BYTES;
    //                 const LINE_MEMORY_SIZE_BYTES: usize = TILES_PER_LINE * FB_SINGLE_TILE_LINE_SIZE;

    //                 // print tile data in vram
    //                 for (tile_num, tile) in gb.video().all_tiles().iter().enumerate() {
    //                     for line_num in 0..TILE_PIXELS {
    //                         let fb_line_num = (tile_num / TILES_PER_LINE) * TILE_PIXELS + line_num;
    //                         let fb_line_start: usize = fb_line_num * LINE_MEMORY_SIZE_BYTES;
    //                         let tile_pos_in_line = tile_num % TILES_PER_LINE;
    //                         let fb_tile_line_start =
    //                             fb_line_start + (tile_pos_in_line * FB_SINGLE_TILE_LINE_SIZE);

    //                         for (i, byte) in tile
    //                             .line(line_num as _, &Palette::MONOCHROME_GREEN)
    //                             .as_bytes()
    //                             .iter()
    //                             .enumerate()
    //                         {
    //                             framebuffer[fb_tile_line_start + i] = *byte;
    //                         }
    //                     }
    //                 }

    //                 // LCD screen
    //                 for (y, line) in gb.video().display().iter().enumerate() {
    //                     let fb_line_num = (8 * 24) + y;
    //                     let fb_lcd_line_start = fb_line_num * LINE_MEMORY_SIZE_BYTES;
    //                     for (x, byte) in line.as_bytes().iter().enumerate() {
    //                         framebuffer[fb_lcd_line_start + x] = *byte;
    //                     }
    //                 }

    //                 println!("printing");
    //                 pixels.render().unwrap();

    //                 //pixels.
    //             }
    //             _ => (),
    //         }
    //     })
    //     .unwrap();
}
