//! Muretto's app icon, for the window and the desktop.
//!
//! X11 takes the icon from the window itself. Wayland compositors look it up
//! instead: they match the window's app id to a desktop entry and draw that
//! entry's icon, so on Linux Muretto installs both into the user's data
//! folder.

use std::sync::Arc;

use image::RgbaImage;

/// The icon as an SVG, and rendered at the sizes desktops ask for.
const SVG: &[u8] = include_bytes!("../assets/icon/muretto.svg");
const PNGS: &[(u32, &[u8])] = &[
    (32, include_bytes!("../assets/icon/muretto-32.png")),
    (48, include_bytes!("../assets/icon/muretto-48.png")),
    (64, include_bytes!("../assets/icon/muretto-64.png")),
    (128, include_bytes!("../assets/icon/muretto-128.png")),
    (256, include_bytes!("../assets/icon/muretto-256.png")),
    (512, include_bytes!("../assets/icon/muretto-512.png")),
];

/// The icon for the window's frame and the task switcher.
pub fn window_icon() -> Option<Arc<RgbaImage>> {
    let (_, png) = PNGS.iter().find(|(size, _)| *size == 128)?;
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
    Some(Arc::new(image.into_rgba8()))
}

/// Puts the icon and a desktop entry where Wayland compositors and app
/// menus find them, unless they are already there. Failing is harmless: the
/// window then shows a generic icon.
#[cfg(target_os = "linux")]
pub fn install_desktop_entry() {
    use std::{fs, path::Path};

    use crate::{APP_ID, APP_NAME};

    fn write_if_changed(path: &Path, contents: &[u8]) -> std::io::Result<()> {
        if fs::read(path).is_ok_and(|current| current == contents) {
            return Ok(());
        }
        fs::create_dir_all(path.parent().expect("the path has a folder"))?;
        fs::write(path, contents)
    }

    let Some(data) = dirs::data_dir() else {
        return;
    };
    let icons = data.join("icons/hicolor");
    let file = format!("{APP_ID}.png");
    let mut result = PNGS.iter().try_for_each(|(size, png)| {
        write_if_changed(&icons.join(format!("{size}x{size}/apps/{file}")), png)
    });
    result = result
        .and_then(|()| write_if_changed(&icons.join(format!("scalable/apps/{APP_ID}.svg")), SVG));
    // Starting Muretto from the app menu runs whichever build last started.
    if let Ok(exe) = std::env::current_exe() {
        let entry = format!(
            "[Desktop Entry]\n\
             Type=Application\n\
             Name={APP_NAME}\n\
             Comment=Launcher and content manager for Assetto Corsa\n\
             Exec=\"{}\"\n\
             Icon={APP_ID}\n\
             Terminal=false\n\
             Categories=Game;\n\
             StartupWMClass={APP_ID}\n",
            exe.display()
        );
        result = result.and_then(|()| {
            write_if_changed(
                &data.join(format!("applications/{APP_ID}.desktop")),
                entry.as_bytes(),
            )
        });
    }
    if let Err(error) = result {
        eprintln!("muretto: could not install the desktop entry: {error}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_size_decodes_at_its_size() {
        for (size, png) in super::PNGS {
            let image = image::load_from_memory(png).expect("the PNG decodes");
            assert_eq!((image.width(), image.height()), (*size, *size));
        }
        assert!(super::window_icon().is_some());
    }
}
