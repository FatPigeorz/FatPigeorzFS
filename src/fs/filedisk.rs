use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::Mutex;
use super::fs::{BlockDevice, BLOCK_SIZE};

pub struct FileDisk(Mutex<File>);

impl FileDisk {
    pub fn new(file: File) -> Self {
        Self(Mutex::new(file))
    }
}

impl BlockDevice for FileDisk {
    fn read_block(&self, block_id: u32, buf: &mut [u8]) {
        let mut file = self.0.lock().unwrap();
        file.seek(SeekFrom::Start((block_id * BLOCK_SIZE) as u64)).unwrap();
        // TODO: async read
        file.read_exact(buf).unwrap();
    }

    fn write_block(&self, block_id: u32, buf: &[u8]) {
        let mut file = self.0.lock().unwrap();
        file.seek(SeekFrom::Start((block_id * BLOCK_SIZE) as u64)).unwrap();
        // TODO: async write
        file.write_all(buf).unwrap();
    }
}

#[allow(unused_imports)]
mod test {
    use std::fs::OpenOptions;

    use super::*;
    #[test]
    fn test_file_disk() {
        // print pwd
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open("./test.img")
            .unwrap();
        file.set_len(1024 * 1024).unwrap();
        file.write_all(&[0; 1024 * 1024]).unwrap();
        let file_disk = FileDisk(Mutex::new(file));
        let mut buf = [0; 512];
        file_disk.write_block(0, &[1; 512]);
        file_disk.read_block(0, &mut buf);
        assert_eq!(buf, [1; 512]);
        file_disk.read_block(1, &mut buf);
        assert_eq!(buf, [0; 512]);
    }
}

