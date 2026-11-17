#[cfg(feature = "android")]
use jni::{
    EnvUnowned,
    objects::{JClass, JObject, JString},
};

pub mod android;
pub mod embed;
pub mod entrypoint;
pub mod vfs;

#[cfg(feature = "android")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Java_com_sbchild_sb_1edge_Main_launchSbEdgeAndroid(
    maybe_env: EnvUnowned,
    _class: JClass,
    context: JObject,
    json_config: JString,
) {
    unsafe { android::jni_entrypoint(maybe_env, _class, context, json_config) }
}
