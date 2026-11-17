#[cfg(feature = "android")]
use jni::{
    EnvUnowned,
    errors::ThrowRuntimeExAndDefault,
    jni_str,
    objects::{JClass, JObject, JString},
    strings::JNIString,
};

#[cfg(feature = "android")]
pub unsafe fn jni_entrypoint(
    mut maybe_env: EnvUnowned,
    _class: JClass,
    context: JObject,
    json_config: JString,
) {
    use crate::entrypoint;
    use std::ffi::c_void;

    eprintln!("=== Into Java Native World ===");
    if context.is_null() {
        eprintln!("Context is null.");
        return;
    }
    let maybe_jvm = maybe_env
        .with_env(|env| {
            let jvm = env.get_java_vm()?;
            Ok::<Option<_>, jni::errors::Error>(Some(jvm))
        })
        .resolve::<ThrowRuntimeExAndDefault>();
    let jvm = match maybe_jvm {
        Some(jvm) => jvm,
        None => {
            eprintln!("Failed to get jvm.");
            return;
        }
    };
    let vm_ptr = jvm.get_raw() as *mut c_void;
    let maybe_global_context = maybe_env
        .with_env(|env| {
            let activity_ptr = env.new_global_ref(&context)?;
            Ok::<Option<_>, jni::errors::Error>(Some(activity_ptr))
        })
        .resolve::<ThrowRuntimeExAndDefault>();
    let global_context = match maybe_global_context {
        Some(global_context) => global_context,
        None => {
            eprintln!("Failed to get global-context.");
            return;
        }
    };
    let activity_ptr = global_context.as_raw() as *mut c_void;
    std::mem::forget(global_context);
    unsafe { ndk_context::initialize_android_context(vm_ptr, activity_ptr) };

    let conf_string = maybe_env
        .with_env(|env| {
            let s = json_config.try_to_string(env)?;
            Ok::<_, jni::errors::Error>(s)
        })
        .resolve::<ThrowRuntimeExAndDefault>();

    if !conf_string.starts_with("{") {
        let err_msg = format!("Failed to get config: {conf_string}");
        eprintln!("{err_msg}");
        let _ = maybe_env
            .with_env(|env| {
                let err_msg = JNIString::new(err_msg);
                env.throw_new(jni_str!("java/lang/IllegalArgumentException"), err_msg)?;
                Ok::<_, jni::errors::Error>(())
            })
            .resolve::<ThrowRuntimeExAndDefault>();
        return;
    }
    let conf: entrypoint::LaunchArgs = match serde_json::from_str(&conf_string) {
        Ok(c) => c,
        Err(e) => {
            let err_msg = format!("Failed to parse config: {e}. String is {conf_string}");
            eprintln!("{err_msg}");
            let _ = maybe_env
                .with_env(|env| {
                    let err_msg = JNIString::new(err_msg);
                    env.throw_new(jni_str!("java/lang/IllegalArgumentException"), err_msg)?;
                    Ok::<_, jni::errors::Error>(())
                })
                .resolve::<ThrowRuntimeExAndDefault>();
            return;
        }
    };
    if let Err(err) = entrypoint::entrypoint(conf) {
        let err_msg = format!("entrypoint error: {:#?}({})", err, err);
        eprintln!("{err_msg}");
        let _ = maybe_env
            .with_env(|env| {
                let err_msg = JNIString::new(err_msg);
                env.throw_new(jni_str!("java/lang/RuntimeException"), err_msg)?;
                Ok::<_, jni::errors::Error>(())
            })
            .resolve::<ThrowRuntimeExAndDefault>();
        return;
    }
    eprintln!("End of Java Native World.");
}
