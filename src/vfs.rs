use snafu::{ResultExt, Snafu};

#[derive(Debug, Snafu)]
pub enum VfsError {
    #[cfg(feature = "android")]
    #[snafu(display("ndk_context JavaVM pointer is null"))]
    NullVm,

    #[cfg(feature = "android")]
    #[snafu(display("ndk_context Context pointer is null"))]
    NullContext,

    #[cfg(feature = "android")]
    #[snafu(display("JNI call failed: {source}"))]
    Jni { source: jni::errors::Error },

    #[cfg(feature = "android")]
    #[snafu(display("Java exception thrown at: {step}"))]
    JavaException { step: &'static str },

    #[snafu(display("I/O error: {source}"))]
    Io { source: std::io::Error },

    #[cfg(feature = "android")]
    #[snafu(display("tzdata parse error: {msg}"))]
    InvalidTzData { msg: &'static str },
}

#[cfg(feature = "android")]
mod android_imp {
    use super::*;
    use jni::{
        Env, JavaVM, jni_sig, jni_str,
        objects::{JObject, JString, JValue},
    };
    use snafu::ensure;
    use std::mem::ManuallyDrop;

    enum FrameError {
        Jni(jni::errors::Error),
        Vfs(VfsError),
    }

    impl From<jni::errors::Error> for FrameError {
        fn from(source: jni::errors::Error) -> Self {
            Self::Jni(source)
        }
    }

    impl From<VfsError> for FrameError {
        fn from(err: VfsError) -> Self {
            Self::Vfs(err)
        }
    }

    impl FrameError {
        fn into_vfs(self) -> VfsError {
            match self {
                Self::Jni(err) => Err::<(), _>(err).context(JniSnafu).unwrap_err(),
                Self::Vfs(err) => err,
            }
        }
    }

    // 使用 Android 的 NDK 上下文
    fn with_android_ctx<T, F>(f: F) -> Result<T, VfsError>
    where
        F: for<'local> FnOnce(&mut Env<'local>, &JObject<'local>) -> Result<T, VfsError>,
    {
        let android_ctx = ndk_context::android_context();

        let vm_ptr = android_ctx.vm();
        ensure!(!vm_ptr.is_null(), NullVmSnafu);

        let ctx_ptr = android_ctx.context();
        ensure!(!ctx_ptr.is_null(), NullContextSnafu);

        // Safety: `jni_entrypoint` 已保证初始化过有效的 `JavaVM` 指针。
        let vm = unsafe { JavaVM::from_raw(vm_ptr.cast()) };

        // 挂载当前线程到 JVM
        vm.attach_current_thread(|env| -> Result<T, FrameError> {
            // 建立局部引用帧
            env.with_local_frame(32, |env| -> Result<T, FrameError> {
                // 借用 `ndk_context` 中的全局 `Context`
                // Safety: `ctx_ptr` 是 `std::mem::forget` 保留的 JNI GlobalRef，
                // 用 `ManuallyDrop` 借用可避免退出作用域时误调用 `DeleteLocalRef`。
                let ctx_obj = ManuallyDrop::new(unsafe { JObject::from_raw(env, ctx_ptr.cast()) });
                Ok(f(env, &ctx_obj)?)
            })
        })
        .map_err(FrameError::into_vfs)
    }

    /// 执行单步 JNI 调用，并拦截异常。
    fn call_jni<'local, T>(
        env: &mut Env<'local>,
        step: &'static str,
        f: impl FnOnce(&mut Env<'local>) -> jni::errors::Result<T>,
    ) -> Result<T, VfsError> {
        match f(env) {
            Ok(val) => {
                // 拦截 Java 侧异常
                if env.exception_check() {
                    // 清理挂起状态
                    env.exception_clear();
                    // 映射到 `VfsError::JavaException`
                    return JavaExceptionSnafu { step }.fail();
                }
                Ok(val)
            }
            Err(jni::errors::Error::JavaException) => {
                if env.exception_check() {
                    env.exception_clear();
                }
                JavaExceptionSnafu { step }.fail()
            }
            Err(err) => Err(err).context(JniSnafu),
        }
    }

    /// 将非空 Java `String` 对象安全转换为 Rust `String`。
    fn jobj_to_string<'local>(
        env: &mut Env<'local>,
        obj: JObject<'local>,
        step: &'static str,
    ) -> Result<String, VfsError> {
        call_jni(env, step, |env| {
            let jstr = JString::cast_local(env, obj)?;
            jstr.try_to_string(env)
        })
    }

    /// 生成 `/etc/resolv.conf` 内容
    pub fn build_resolv_conf() -> Result<Vec<u8>, VfsError> {
        with_android_ctx(|env, ctx| {
            let svc_name = call_jni(env, "JString::from_jni_str(connectivity)", |env| {
                JString::from_jni_str(env, jni_str!("connectivity"))
            })?;

            let cm = call_jni(env, "Context.getSystemService(connectivity)", |env| {
                env.call_method(
                    ctx,
                    jni_str!("getSystemService"),
                    jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"),
                    &[JValue::Object(&svc_name)],
                )?
                .l()
            })?;

            let mut dns_ips = Vec::new();
            let mut search_domains = String::new();

            if !cm.is_null() {
                let network = call_jni(env, "ConnectivityManager.getActiveNetwork", |env| {
                    env.call_method(
                        &cm,
                        jni_str!("getActiveNetwork"),
                        jni_sig!("()Landroid/net/Network;"),
                        &[],
                    )?
                    .l()
                })?;

                if !network.is_null() {
                    let link_props =
                        call_jni(env, "ConnectivityManager.getLinkProperties", |env| {
                            env.call_method(
                                &cm,
                                jni_str!("getLinkProperties"),
                                jni_sig!("(Landroid/net/Network;)Landroid/net/LinkProperties;"),
                                &[JValue::Object(&network)],
                            )?
                            .l()
                        })?;

                    if !link_props.is_null() {
                        let domains_obj = call_jni(env, "LinkProperties.getDomains", |env| {
                            env.call_method(
                                &link_props,
                                jni_str!("getDomains"),
                                jni_sig!("()Ljava/lang/String;"),
                                &[],
                            )?
                            .l()
                        })?;
                        if !domains_obj.is_null() {
                            search_domains =
                                jobj_to_string(env, domains_obj, "LinkProperties.getDomains->str")?;
                        }

                        let dns_list = call_jni(env, "LinkProperties.getDnsServers", |env| {
                            env.call_method(
                                &link_props,
                                jni_str!("getDnsServers"),
                                jni_sig!("()Ljava/util/List;"),
                                &[],
                            )?
                            .l()
                        })?;

                        if !dns_list.is_null() {
                            let size = call_jni(env, "List.size", |env| {
                                env.call_method(&dns_list, jni_str!("size"), jni_sig!("()I"), &[])?
                                    .i()
                            })?;

                            for idx in 0..size {
                                let inet = call_jni(env, "List.get", |env| {
                                    env.call_method(
                                        &dns_list,
                                        jni_str!("get"),
                                        jni_sig!("(I)Ljava/lang/Object;"),
                                        &[JValue::Int(idx)],
                                    )?
                                    .l()
                                })?;
                                if inet.is_null() {
                                    continue;
                                }

                                let addr_obj =
                                    call_jni(env, "InetAddress.getHostAddress", |env| {
                                        env.call_method(
                                            &inet,
                                            jni_str!("getHostAddress"),
                                            jni_sig!("()Ljava/lang/String;"),
                                            &[],
                                        )?
                                        .l()
                                    })?;
                                if !addr_obj.is_null() {
                                    let ip = jobj_to_string(
                                        env,
                                        addr_obj,
                                        "InetAddress.getHostAddress->str",
                                    )?;
                                    let trimmed = ip.trim();
                                    if !trimmed.is_empty() {
                                        dns_ips.push(trimmed.to_owned());
                                    }
                                }
                            }
                        }
                    }
                }
            }

            let mut out = String::from("# Generated by sb-edge Android VFS\n");
            let domains: Vec<&str> = search_domains
                .split(|c: char| c == ',' || c.is_ascii_whitespace())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .collect();
            if !domains.is_empty() {
                out.push_str("search ");
                out.push_str(&domains.join(" "));
                out.push('\n');
            }

            if dns_ips.is_empty() {
                out.push_str("nameserver 1.1.1.1\nnameserver 8.8.8.8\n");
            } else {
                for ip in &dns_ips {
                    out.push_str("nameserver ");
                    out.push_str(ip);
                    out.push('\n');
                }
            }
            out.push_str("options ndots:1 timeout:2 attempts:2\n");
            Ok(out.into_bytes())
        })
    }

    /// 生成 `/etc/ssl/certs/ca-certificates.crt` (PEM Bundle) 内容
    pub fn build_ca_certificates_crt() -> Result<Vec<u8>, VfsError> {
        with_android_ctx(|env, _ctx| {
            let store_name = call_jni(env, "JString::from_jni_str(AndroidCAStore)", |env| {
                JString::from_jni_str(env, jni_str!("AndroidCAStore"))
            })?;

            let ks = call_jni(env, "KeyStore.getInstance(AndroidCAStore)", |env| {
                env.call_static_method(
                    jni_str!("java/security/KeyStore"),
                    jni_str!("getInstance"),
                    jni_sig!("(Ljava/lang/String;)Ljava/security/KeyStore;"),
                    &[JValue::Object(&store_name)],
                )?
                .l()
            })?;

            let null_obj = JObject::null();
            call_jni(env, "KeyStore.load", |env| {
                env.call_method(
                    &ks,
                    jni_str!("load"),
                    jni_sig!("(Ljava/io/InputStream;[C)V"),
                    &[JValue::Object(&null_obj), JValue::Object(&null_obj)],
                )?;
                Ok(())
            })?;

            let aliases = call_jni(env, "KeyStore.aliases", |env| {
                env.call_method(
                    &ks,
                    jni_str!("aliases"),
                    jni_sig!("()Ljava/util/Enumeration;"),
                    &[],
                )?
                .l()
            })?;

            let mut pem_bundle = String::with_capacity(256 * 1024);

            loop {
                let has_more = call_jni(env, "Enumeration.hasMoreElements", |env| {
                    env.call_method(&aliases, jni_str!("hasMoreElements"), jni_sig!("()Z"), &[])?
                        .z()
                })?;
                if !has_more {
                    break;
                }

                // 每次循环单独建立局部引用帧，避免百余张根证书撑爆 ART 的 512 LocalRef 上限
                env.with_local_frame(16, |env| -> Result<(), FrameError> {
                    let alias = call_jni(env, "Enumeration.nextElement", |env| {
                        env.call_method(
                            &aliases,
                            jni_str!("nextElement"),
                            jni_sig!("()Ljava/lang/Object;"),
                            &[],
                        )?
                        .l()
                    })?;
                    if alias.is_null() {
                        return Ok(());
                    }

                    let cert = call_jni(env, "KeyStore.getCertificate", |env| {
                        env.call_method(
                            &ks,
                            jni_str!("getCertificate"),
                            jni_sig!("(Ljava/lang/String;)Ljava/security/cert/Certificate;"),
                            &[JValue::Object(&alias)],
                        )?
                        .l()
                    })?;
                    if cert.is_null() {
                        return Ok(());
                    }

                    let der_obj = call_jni(env, "Certificate.getEncoded", |env| {
                        env.call_method(&cert, jni_str!("getEncoded"), jni_sig!("()[B"), &[])?
                            .l()
                    })?;
                    if der_obj.is_null() {
                        return Ok(());
                    }

                    // android.util.Base64.NO_WRAP == 2
                    let b64_obj = call_jni(env, "Base64.encodeToString", |env| {
                        env.call_static_method(
                            jni_str!("android/util/Base64"),
                            jni_str!("encodeToString"),
                            jni_sig!("([BI)Ljava/lang/String;"),
                            &[JValue::Object(&der_obj), JValue::Int(2)],
                        )?
                        .l()
                    })?;
                    if b64_obj.is_null() {
                        return Ok(());
                    }

                    let b64_raw = jobj_to_string(env, b64_obj, "Base64.encodeToString->str")?;
                    if !b64_raw.is_empty() {
                        pem_bundle.push_str("-----BEGIN CERTIFICATE-----\n");
                        for chunk in b64_raw.as_bytes().chunks(64) {
                            if let Ok(line) = std::str::from_utf8(chunk) {
                                pem_bundle.push_str(line);
                                pem_bundle.push('\n');
                            }
                        }
                        pem_bundle.push_str("-----END CERTIFICATE-----\n");
                    }
                    Ok(())
                })
                .map_err(FrameError::into_vfs)?;
            }

            Ok(pem_bundle.into_bytes())
        })
    }

    /// 生成 `/etc/localtime` (RFC 8536 TZif 二进制格式) 内容
    pub fn build_localtime() -> Result<Vec<u8>, VfsError> {
        let tz_id = with_android_ctx(|env, _ctx| {
            let tz_obj = call_jni(env, "TimeZone.getDefault", |env| {
                env.call_static_method(
                    jni_str!("java/util/TimeZone"),
                    jni_str!("getDefault"),
                    jni_sig!("()Ljava/util/TimeZone;"),
                    &[],
                )?
                .l()
            })?;
            if tz_obj.is_null() {
                return Ok(String::from("Etc/UTC"));
            }

            let id_obj = call_jni(env, "TimeZone.getID", |env| {
                env.call_method(
                    &tz_obj,
                    jni_str!("getID"),
                    jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()
            })?;
            if id_obj.is_null() {
                return Ok(String::from("Etc/UTC"));
            }

            jobj_to_string(env, id_obj, "TimeZone.getID->str")
        })?;

        let tzdata_bytes = read_bionic_tzdata()?;
        let normalized_id = match tz_id.trim() {
            "" | "GMT" | "UTC" => "Etc/UTC",
            other => other,
        };
        extract_tzif_from_bionic_tzdata(&tzdata_bytes, normalized_id)
    }

    fn read_bionic_tzdata() -> Result<Vec<u8>, VfsError> {
        const APEX_TZDATA: &str = "/apex/com.android.tzdata/etc/tz/tzdata";
        const SYSTEM_TZDATA: &str = "/system/usr/share/zoneinfo/tzdata";

        match std::fs::read(APEX_TZDATA) {
            Ok(bytes) => Ok(bytes),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                std::fs::read(SYSTEM_TZDATA).context(IoSnafu)
            }
            Err(err) => Err(err).context(IoSnafu),
        }
    }

    fn extract_tzif_from_bionic_tzdata(bytes: &[u8], tz_id: &str) -> Result<Vec<u8>, VfsError> {
        ensure!(
            bytes.len() >= 24,
            InvalidTzDataSnafu {
                msg: "tzdata file shorter than 24-byte header",
            }
        );
        ensure!(
            bytes.starts_with(b"tzdata") && bytes[11] == 0,
            InvalidTzDataSnafu {
                msg: "invalid tzdata magic header",
            }
        );

        let index_offset = i32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let data_offset = i32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);

        ensure!(
            index_offset >= 24 && data_offset >= index_offset,
            InvalidTzDataSnafu {
                msg: "negative or inverted tzdata section offsets",
            }
        );

        let idx_start = index_offset as usize;
        let idx_end = data_offset as usize;
        ensure!(
            idx_end <= bytes.len() && (idx_end - idx_start) % 52 == 0,
            InvalidTzDataSnafu {
                msg: "tzdata index section out of bounds or misaligned",
            }
        );

        let target = tz_id.as_bytes();
        for entry in bytes[idx_start..idx_end].chunks_exact(52) {
            let raw_name = &entry[0..40];
            let name_len = raw_name.iter().position(|&b| b == 0).unwrap_or(40);
            if &raw_name[..name_len] == target {
                let rel_start = i32::from_be_bytes([entry[40], entry[41], entry[42], entry[43]]);
                let length = i32::from_be_bytes([entry[44], entry[45], entry[46], entry[47]]);
                ensure!(
                    rel_start >= 0 && length > 0,
                    InvalidTzDataSnafu {
                        msg: "invalid entry offset or length in tzdata index",
                    }
                );

                let tzif_start = idx_end
                    .checked_add(rel_start as usize)
                    .filter(|&s| s <= bytes.len());
                let tzif_end = tzif_start
                    .and_then(|s| s.checked_add(length as usize))
                    .filter(|&e| e <= bytes.len());

                let (Some(start), Some(end)) = (tzif_start, tzif_end) else {
                    return InvalidTzDataSnafu {
                        msg: "tzif payload slice exceeds tzdata file size",
                    }
                    .fail();
                };

                let slice = &bytes[start..end];
                ensure!(
                    slice.starts_with(b"TZif"),
                    InvalidTzDataSnafu {
                        msg: "extracted payload missing TZif magic",
                    }
                );
                return Ok(slice.to_vec());
            }
        }

        InvalidTzDataSnafu {
            msg: "requested timezone ID not found in tzdata index",
        }
        .fail()
    }

    /// 生成 `/etc/hosts` 内容
    pub fn build_hosts() -> Result<Vec<u8>, VfsError> {
        let hostname = with_android_ctx(|env, _ctx| {
            let local_host = call_jni(env, "InetAddress.getLocalHost", |env| {
                env.call_static_method(
                    jni_str!("java/net/InetAddress"),
                    jni_str!("getLocalHost"),
                    jni_sig!("()Ljava/net/InetAddress;"),
                    &[],
                )?
                .l()
            })?;
            if local_host.is_null() {
                return Ok(String::new());
            }

            let name_obj = call_jni(env, "InetAddress.getHostName", |env| {
                env.call_method(
                    &local_host,
                    jni_str!("getHostName"),
                    jni_sig!("()Ljava/lang/String;"),
                    &[],
                )?
                .l()
            })?;
            if name_obj.is_null() {
                return Ok(String::new());
            }

            jobj_to_string(env, name_obj, "InetAddress.getHostName->str")
        })?;

        let mut out = String::from(concat!(
            "# Generated by sb-edge Android VFS\n",
            "127.0.0.1\tlocalhost\n",
            "::1\t\tlocalhost ip6-localhost ip6-loopback\n",
        ));

        let clean_host = hostname.trim();
        if !clean_host.is_empty()
            && clean_host != "localhost"
            && clean_host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        {
            out.push_str("127.0.0.1\t");
            out.push_str(clean_host);
            out.push('\n');
            out.push_str("::1\t\t");
            out.push_str(clean_host);
            out.push('\n');
        }

        Ok(out.into_bytes())
    }
}

#[cfg(not(feature = "android"))]
mod host_imp {
    use super::*;

    /// 读取 `/etc/resolv.conf` 内容
    pub fn build_resolv_conf() -> Result<Vec<u8>, VfsError> {
        std::fs::read("/etc/resolv.conf").context(IoSnafu)
    }

    /// 读取 `/etc/ssl/certs/ca-certificates.crt` 内容
    pub fn build_ca_certificates_crt() -> Result<Vec<u8>, VfsError> {
        // 对齐 Go 标准库 crypto/x509/root_linux.go 的检索顺序（同时兼容 Debian/Ubuntu 与 Fedora/RHEL）
        const CERT_FILES: &[&str] = &[
            "/etc/ssl/certs/ca-certificates.crt",
            "/etc/pki/tls/certs/ca-bundle.crt",
            "/etc/ssl/ca-bundle.pem",
            "/etc/pki/tls/cacert.pem",
            "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem",
            "/etc/ssl/cert.pem",
        ];

        let mut last_err = None;
        for path in CERT_FILES {
            match std::fs::read(path) {
                Ok(bytes) => return Ok(bytes),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    last_err = Some(err);
                }
                Err(err) => return Err(err).context(IoSnafu),
            }
        }
        Err(last_err.unwrap_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "no host CA bundle found")
        }))
        .context(IoSnafu)
    }

    /// 读取 `/etc/localtime` 内容
    pub fn build_localtime() -> Result<Vec<u8>, VfsError> {
        std::fs::read("/etc/localtime").context(IoSnafu)
    }

    /// 读取 `/etc/hosts` 内容
    pub fn build_hosts() -> Result<Vec<u8>, VfsError> {
        std::fs::read("/etc/hosts").context(IoSnafu)
    }
}

#[cfg(feature = "android")]
pub use android_imp::{build_ca_certificates_crt, build_hosts, build_localtime, build_resolv_conf};

#[cfg(not(feature = "android"))]
pub use host_imp::{build_ca_certificates_crt, build_hosts, build_localtime, build_resolv_conf};
