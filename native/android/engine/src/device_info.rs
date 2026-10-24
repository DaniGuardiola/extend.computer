//! Presentation-only hardware classification from the configured application resources.
use extend_computer_agent::device_info::DeviceKind;
use jni::{objects::JObject, JNIEnv};

pub(super) fn detect(env: &mut JNIEnv, context: &JObject) -> DeviceKind {
    let width = (|| -> jni::errors::Result<i32> {
        let resources = env
            .call_method(
                context,
                "getResources",
                "()Landroid/content/res/Resources;",
                &[],
            )?
            .l()?;
        let configuration = env
            .call_method(
                resources,
                "getConfiguration",
                "()Landroid/content/res/Configuration;",
                &[],
            )?
            .l()?;
        env.get_field(configuration, "smallestScreenWidthDp", "I")?
            .i()
    })();
    match width {
        Ok(width) => classify(width),
        Err(_) => {
            // Icon metadata is optional. A failed OS query must not prevent
            // engine initialization or leave a pending Java exception behind.
            let _ = env.exception_clear();
            DeviceKind::Unknown
        }
    }
}

fn classify(smallest_width_dp: i32) -> DeviceKind {
    match smallest_width_dp {
        ..=0 => DeviceKind::Unknown,
        1..600 => DeviceKind::Phone,
        _ => DeviceKind::Tablet,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configured_screen_size_distinguishes_phone_tablet_and_missing_dimensions() {
        assert_eq!(classify(0), DeviceKind::Unknown);
        assert_eq!(classify(-1), DeviceKind::Unknown);
        assert_eq!(classify(360), DeviceKind::Phone);
        assert_eq!(classify(599), DeviceKind::Phone);
        assert_eq!(classify(600), DeviceKind::Tablet);
        assert_eq!(classify(800), DeviceKind::Tablet);
    }
}
