pub struct RomInfo {
    title: String
}

impl RomInfo {
    pub fn new(rom: &[u8]) -> RomInfo {
         let mut title = String::new();
         for character in rom.slice(0x134, 0x144).iter() {
             if *character == 0u8 {
                 break;
             }

             title.push_char(*character as char)
         }

         RomInfo {
             title: title
         }
    }

    pub fn title(&self) -> &str {
        self.title.as_slice()
    }
}
