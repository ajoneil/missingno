pub struct Video {
    lcdc: u8,
    stat: u8,
    scroll_x: u8,
    scroll_y: u8,
    ly: u8,
    lyc: u8,
    bgp: u8,
    obp0: u8,
    obp1: u8
}

impl Video {
    pub fn new() -> Video {
        Video {
            lcdc: 0x91,
            stat: 0,
            scroll_x: 0,
            scroll_y: 0,
            ly: 0,
            lyc: 0,
            bgp: 0xfc,
            obp0: 0xff,
            obp1: 0xff
        }
    }

    pub fn read(&self, address: u16) -> u8 {
        match address {
            0xff40 => self.lcdc,
            0xff41 => self.stat,
            0xff42 => self.scroll_y,
            0xff43 => self.scroll_x,
            0xff44 => self.ly,
            0xff45 => self.lyc,
            0xff47 => self.bgp,
            0xff48 => self.obp0,
            0xff49 => self.obp1,
            _ => fail!("Unimplemented video read from {:x}", address)
        }
    }

    pub fn write(&mut self, address: u16, val: u8) {
        match address {
            0xff40 => self.lcdc = val,
            0xff41 => self.stat = val,
            0xff42 => self.scroll_y = val,
            0xff43 => self.scroll_x = val,
            0xff45 => self.lyc = val,
            0xff47 => self.bgp = val,
            0xff48 => self.obp0 = val,
            0xff49 => self.obp1 = val,
            _ => fail!("Unimplemented video write to {:x}", address)
        }
    }
}
