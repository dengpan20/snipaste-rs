//! 持有剪贴板连接；完成截图仅在图像写入成功之后关闭遮罩。
use std::borrow::Cow;
pub struct Clipboard(arboard::Clipboard);
impl Clipboard {
    pub fn new() -> Result<Self, arboard::Error> {
        arboard::Clipboard::new().map(Self)
    }
    pub fn set_image(
        &mut self,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), arboard::Error> {
        self.0.set_image(arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: Cow::Borrowed(rgba),
        })
    }
    pub fn image(&mut self) -> Result<(u32, u32, Vec<u8>), arboard::Error> {
        let img = self.0.get_image()?;
        Ok((img.width as u32, img.height as u32, img.bytes.into_owned()))
    }
    pub fn set_text(&mut self, text: &str) -> Result<(), arboard::Error> {
        self.0.set_text(text)
    }
    pub fn text(&mut self) -> Result<String, arboard::Error> {
        self.0.get_text()
    }
}
