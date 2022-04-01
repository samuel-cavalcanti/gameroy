use std::{collections::HashMap, rc::Rc};

use giui::{
    font::{Font, Fonts},
    graphics::{Graphic, TextStyle},
    style::{ButtonStyle, TabStyle, TextFieldStyle},
    style_loader::{load_style, StyleLoaderCallback},
};
use sprite_render::SpriteRender;

use crate::fold_view::FoldIcon;

struct Loader<'a, R: SpriteRender> {
    fonts: &'a mut Fonts,
    render: &'a mut R,
    textures: HashMap<String, (u32, u32, u32)>,
}
impl<'a, R: SpriteRender> StyleLoaderCallback for Loader<'a, R> {
    fn load_texture(&mut self, name: String) -> (u32, u32, u32) {
        if let Some(texture) = self.textures.get(&name) {
            return *texture;
        }

        let path = format!("assets/{}", name);
        let data = match image::open(&path) {
            Ok(x) => x,
            Err(_) => {
                log::error!("not found texture in '{}'", path);
                return (0, 0, 0);
            }
        };
        let data = data.to_rgba8();

        let texture = (
            self.render
                .new_texture(data.width(), data.height(), data.as_ref(), true),
            data.width(),
            data.height(),
        );
        self.textures.insert(name, texture);
        texture
    }

    fn load_font(&mut self, name: String) -> giui::font::FontId {
        // load a font
        let path = "assets/".to_string() + &name;
        log::info!("load font: '{}'", path);
        let font_data = std::fs::read(path).unwrap();
        self.fonts.add(Font::new(&font_data))
    }
}

#[derive(LoadStyle, Clone)]
pub struct Style {
    pub text_style: TextStyle,
    pub split_background: Graphic,
    pub terminal_background: Graphic,
    pub terminal_text_style: TextStyle,
    pub background: Graphic,
    pub header_background: Graphic,
    pub text_field: Rc<TextFieldStyle>,
    pub scrollbar: Rc<ButtonStyle>,
    pub delete_button: Rc<ButtonStyle>,
    pub tab_style: Rc<TabStyle>,
    pub fold_icon: FoldIcon,
    pub delete_icon: Graphic,
}
impl Style {
    pub fn load(fonts: &mut Fonts, render: &mut impl SpriteRender) -> Option<Self> {
        let loader = Loader {
            fonts,
            render,
            textures: HashMap::default(),
        };
        let file = std::fs::read_to_string("assets/style.ron").unwrap();
        let mut deser = ron::Deserializer::from_str(&file).unwrap();
        let style: Result<Self, _> = load_style(&mut deser, loader);

        Some(style.unwrap())
    }
}
