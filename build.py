import argparse
import base64
import hashlib
import json
import os
import re
import shlex
import shutil
import struct
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Literal
from packaging.version import Version

# 构建脚本所在目录
SCRIPT_DIR = Path(__file__).parent.resolve()
# 构建成果目录
BUILD_BASE = SCRIPT_DIR / "build"
# 临时目录
BUILD_TEMP = SCRIPT_DIR / "build" / "temp"
# crate名字
APP_NAME = "sb-edge"
APP_NAME_NORM = APP_NAME.replace("-", "_")

PlatformType = Literal["linux", "macos", "windows"]
LibType = Literal["cdylib", "staticlib"]


def crate_to_lib_name(
    crate_name: str, platform: PlatformType = "linux", lib_type: LibType = "cdylib"
) -> str:
    cleaned_name = crate_name.strip()
    if not cleaned_name or not re.match(r"^[a-zA-Z0-9_-]+$", cleaned_name):
        raise ValueError(f"无效的 Rust crate 名称: '{crate_name}'")
    norm_name = cleaned_name.replace("-", "_")
    rules = {
        "linux": {"cdylib": ("lib", ".so"), "staticlib": ("lib", ".a")},
        "macos": {"cdylib": ("lib", ".dylib"), "staticlib": ("lib", ".a")},
        "windows": {"cdylib": ("", ".dll"), "staticlib": ("", ".lib")},
    }
    if platform not in rules:
        raise ValueError(f"不支持的平台类型: {platform}")
    if lib_type not in rules[platform]:
        raise ValueError(f"不支持的库类型: {lib_type}")
    prefix, suffix = rules[platform][lib_type]
    return f"{prefix}{norm_name}{suffix}"


def lib_name(platform: PlatformType) -> str:
    return crate_to_lib_name(APP_NAME, platform, "cdylib")


def init():
    # 创建构建所需目录
    for d in [BUILD_BASE, BUILD_TEMP]:
        d.mkdir(exist_ok=True, parents=True)
    print(f"已创建构建目录: {BUILD_BASE}")
    # 删除临时文件
    for i in BUILD_TEMP.iterdir():
        if i.is_dir() and not i.is_symlink():
            shutil.rmtree(i)
        else:
            i.unlink()
    print(f"已清除临时文件: {BUILD_TEMP}")
    tf = BUILD_TEMP / "_tempfile"
    tf.mkdir(exist_ok=True)
    tempfile.tempdir = str(tf)
    # 确保已经安装 cargo-update
    ensure_cargo_update_installed()
    ensure_javac_installed()


def mktempdir() -> Path:
    return Path(tempfile.mkdtemp()).resolve()


def ship(f: Path, target: str) -> Path:
    s = f"{APP_NAME}_{target}"
    r = f.move_into(BUILD_BASE)
    r = r.rename(BUILD_BASE / s)
    return r


# ---

# 8 字节魔数
EMBED_MAGIC = b"sb-edge-"
# 64 位无符号整数 = 8 字节
EMBED_LEN_FMT = "<Q"
# 16 字节
EMBED_TRAILER_LEN = struct.calcsize(EMBED_LEN_FMT) + len(EMBED_MAGIC)


def split_bin(exe: Path):
    with open(exe, "rb") as f:
        d = f.read()
    if len(d) >= EMBED_TRAILER_LEN and d[-len(EMBED_MAGIC) :] == EMBED_MAGIC:
        n = struct.unpack(EMBED_LEN_FMT, d[-EMBED_TRAILER_LEN : -len(EMBED_MAGIC)])[0]
        if n + EMBED_TRAILER_LEN <= len(d):
            return d[: len(d) - EMBED_TRAILER_LEN - n], d[
                len(d) - EMBED_TRAILER_LEN - n : len(d) - EMBED_TRAILER_LEN
            ]
    return d, b""


def embed_bin(exe: Path, payload: bytes):
    base, split = split_bin(exe)
    print(
        f"base {len(base)} bytes, split {len(split)} bytes, replace {len(payload)} bytes"
    )
    with open(exe, "wb") as f:
        f.write(base)
        if payload:
            f.write(payload)
            f.write(struct.pack(EMBED_LEN_FMT, len(payload)))
            f.write(EMBED_MAGIC)


# ---


def ensure_cargo_update_installed():
    try:
        r = subprocess.run(
            ["cargo", "install-update", "-V"],
            capture_output=True,
            text=True,
            check=True,
        )
    except subprocess.CalledProcessError as e:
        print(f"环境检查失败({e.returncode}):\n{e.stderr.strip()}")
        print("请执行: `cargo install cargo-update`")
        raise e
    print(f"已安装: {r.stdout.strip()}")


def ensure_javac_installed():
    try:
        r = subprocess.run(
            ["javac", "--version"],
            capture_output=True,
            text=True,
            check=True,
        )
    except subprocess.CalledProcessError as e:
        print(f"环境检查失败({e.returncode}):\n{e.stderr.strip()}")
        print("请安装 openjdk 开发包。")
        raise e
    print(f"已安装: {r.stdout.strip()}")


def _cargo_install(crate_name: str, upgrade: bool):
    if not upgrade:
        cmd = ["cargo", "install", crate_name]
    else:
        cmd = ["cargo", "install-update", crate_name]
    try:
        _r = subprocess.run(cmd, check=True)
    except subprocess.CalledProcessError as e:
        action = "升级" if upgrade else "安装"
        print(f"{action} {crate_name} 失败({e.returncode}):\n{e.stderr.strip()}")
        raise e


def cargo_install(crate_name: str, min_version: str):
    try:
        result = subprocess.run(
            ["cargo", "install", "--list"],
            capture_output=True,
            text=True,
            check=True,
        )
    except subprocess.CalledProcessError as e:
        print(f"无法列出 crate 列表({e.returncode}):\n{e.stderr.strip()}")
        raise e
    installed_version = None
    pattern = re.compile(
        rf"^{re.escape(crate_name)}\s+v([0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?):",
        re.MULTILINE,
    )
    match = pattern.search(result.stdout)
    if match:
        installed_version = match.group(1)
    if installed_version is None:
        print(f"正在安装 crate: {crate_name}")
        _cargo_install(crate_name, upgrade=False)
        return
    print(f"crate {crate_name} 当前版本 v{installed_version}。")
    if Version(installed_version) < Version(min_version):
        print(
            f"crate {crate_name} v{installed_version}，不满足至少 v{min_version}，正在升级。"
        )
        _cargo_install(crate_name, upgrade=True)
        return
    print(f"crate {crate_name} v{installed_version} 满足最小版本 v{min_version} 要求。")


def rustup_target_install(target_name: str):
    print(f"正在安装 target: {target_name}")
    try:
        _r = subprocess.run(
            ["rustup", "target", "install", target_name],
            check=True,
        )
    except subprocess.CalledProcessError as e:
        print(f"target {target_name} 安装失败")
        raise e


def _get_ndk_env(abi: str, platform: str) -> dict[str, str]:
    cmd = ["cargo", "ndk-env", "--target", abi, "--platform", platform]
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, check=True)
        output = result.stdout
    except (subprocess.SubprocessError, FileNotFoundError) as e:
        raise RuntimeError(f"cargo ndk-env 执行失败: {e}") from e
    env_vars: dict[str, str] = {}
    for line in output.splitlines():
        line = line.strip()
        if not line or line.startswith("#") or not line.startswith("export "):
            continue
        kv_pair = line[len("export ") :].strip()
        if "=" in kv_pair:
            key, raw_value = kv_pair.split("=", 1)
            key = key.strip()
            try:
                parsed_tokens = shlex.split(raw_value)
                value = parsed_tokens[0] if parsed_tokens else ""
            except ValueError:
                value = raw_value.strip("\"'")
            env_vars[key] = value
    return env_vars


def get_android_ndk_home(abi: str, platform: str) -> Path:
    env_vars = _get_ndk_env(abi, platform)
    path_candidates = [
        env_vars.get("CARGO_NDK_SYSROOT_PATH"),
        env_vars.get("CLANG_PATH"),
        env_vars.get("CARGO_NDK_SYSROOT_LIBS_PATH"),
    ]
    for path_str in path_candidates:
        if not path_str:
            continue
        if "/toolchains/" in path_str:
            ndk_root = path_str.partition("/toolchains/")[0]
            return Path(ndk_root.rstrip("/") + "/").resolve()
    for val in env_vars.values():
        if val and "/toolchains/" in val:
            ndk_root = val.partition("/toolchains/")[0]
            return Path(ndk_root.rstrip("/") + "/").resolve()
    raise ValueError(f"找不到 abi={abi} 的 Android NDK")


def get_android_sdk_home() -> Path:
    env = os.environ
    android_home = env.get("ANDROID_HOME")
    if android_home is None:
        raise FileNotFoundError("找不到 ANDROID_HOME 环境变量")
    p = Path(android_home).resolve()
    assert (p / "platforms").is_dir(), "$ANDROID_HOME/platforms 目录应该存在"
    return Path(android_home).resolve()


def ensure_android_sdk_platform(platform: str):
    h = get_android_sdk_home()
    bootclass = h / "platforms" / f"android-{platform}" / "android.jar"
    assert bootclass.is_file(), f"平台版本 {platform} 的 android.jar 文件应该存在"
    build_tools_version = f"{platform}.0.0"
    d8 = h / "build-tools" / build_tools_version / "d8"
    assert d8.is_file(), f"构建工具版本 {build_tools_version} 的 d8 文件应该存在"


# ---


def build_android_java_entrypoint(platform: str):
    print(f"正在为平台 {platform} 构建 Java 入口点")
    java_version = "8"
    build_tools_version = f"{platform}.0.0"
    android_home = get_android_sdk_home()
    bootclass = android_home / "platforms" / f"android-{platform}" / "android.jar"
    main_file = SCRIPT_DIR / "android" / "Main.java"
    output_dir = mktempdir()
    cmd_header = [
        "javac",
        "-source",
        java_version,
        "-target",
        java_version,
        "-Xlint:-options",
    ]
    cmd_bootclass = ["-bootclasspath", str(bootclass.resolve())]
    cmd_files = ["-d", str(output_dir.resolve()), str(main_file.resolve())]
    try:
        _r = subprocess.run(
            cmd_header + cmd_bootclass + cmd_files,
            check=True,
            env=os.environ,
        )
    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"构建 Java 入口点失败: {e}")
    cmd_executable = (
        android_home / "build-tools" / build_tools_version / "d8"
    ).resolve()
    main_class = (
        output_dir / "com" / "sbchild" / APP_NAME_NORM / "Main.class"
    ).resolve()
    cmd_files = [
        main_class,
        "--output",
        (output_dir / f"{APP_NAME_NORM}_dex.jar").resolve(),
    ]
    try:
        _r = subprocess.run(
            [cmd_executable] + cmd_files,
            check=True,
            env=os.environ,
        )
    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"无法转换 Java 入口点到 dex: {e}")
    res = (output_dir / f"{APP_NAME_NORM}_dex.jar").resolve()
    if not res.is_file():
        raise FileNotFoundError(f"找不到 {res} 文件")
    return res


def _abi_to_android_target(abi: str) -> str:
    m = {
        "arm64-v8a": "aarch64-linux-android",
        "armeabi-v7a": "armv7-linux-androideabi",
        "x86": "i686-linux-android",
        "x86_64": "x86_64-linux-android",
    }
    return m[abi]


def _abi_to_linux_target(abi: str) -> str:
    m = {
        "arm64-v8a": "aarch64-unknown-linux-gnu",
        "armeabi-v7a": "armv7-unknown-linux-gnueabi",
        "x86": "i686-unknown-linux-gnu",
        "x86_64": "x86_64-unknown-linux-gnu",
    }
    return m[abi]


def build_android_binary(platform: str, abi: str, ndk_home: Path):
    # 假设 cargo-ndk 和 target 都装了
    rust_target = _abi_to_android_target(abi)
    env = os.environ.copy() | {
        # https://github.com/DoumanAsh/opusic-sys#android-build
        "ANDROID_NDK_HOME": ndk_home
    }
    try:
        _r = subprocess.run(
            [
                "cargo",
                "ndk",
                "--platform",
                platform,
                "-t",
                abi,
                "build",
                "--bin",
                APP_NAME,
                "--release",
                "--no-default-features",
                "--features",
                "android",
            ],
            check=True,
            env=env,
        )
    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"构建程序 {APP_NAME} 失败: {e}")
    res = (SCRIPT_DIR / "target" / rust_target / "release" / APP_NAME).resolve()
    if not res.is_file():
        raise FileNotFoundError(f"找不到构建产物 {res}")
    r = res.move_into(mktempdir())
    return r


def build_android_library(platform: str, abi: str, ndk_home: Path):
    # 假设 cargo-ndk 和 target 都装了
    rust_target = _abi_to_android_target(abi)
    _lib_name = lib_name("linux")
    env = os.environ.copy() | {
        # https://github.com/DoumanAsh/opusic-sys#android-build
        "ANDROID_NDK_HOME": ndk_home
    }
    try:
        _r = subprocess.run(
            [
                "cargo",
                "ndk",
                "--platform",
                platform,
                "-t",
                abi,
                "build",
                "--lib",
                "--release",
                "--no-default-features",
                "--features",
                "android",
            ],
            check=True,
            env=env,
        )
    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"构建动态库 {_lib_name} 失败: {e}")
    res = (SCRIPT_DIR / "target" / rust_target / "release" / _lib_name).resolve()
    if not res.is_file():
        raise FileNotFoundError(f"找不到构建产物 {res}")
    r = res.move_into(mktempdir())
    return r


def build_android(sdk_platform: str, abi: str):
    cargo_install("cargo-ndk", "4.1.2")
    target = _abi_to_android_target(abi)
    rustup_target_install(target)
    ensure_android_sdk_platform(sdk_platform)
    ndk_home = get_android_ndk_home(abi, sdk_platform)
    print("NDK:", ndk_home)
    sdk_home = get_android_sdk_home()
    print("SDK:", sdk_home)
    entrypoint = build_android_java_entrypoint(sdk_platform)
    print("Entrypoint:", entrypoint)
    android_library = build_android_library(sdk_platform, abi, ndk_home)
    print("Android Library:", android_library)
    android_binary = build_android_binary(sdk_platform, abi, ndk_home)
    print("Android Binary:", android_binary)
    embed_pack = {}
    with open(entrypoint, "rb") as f:
        digest = hashlib.file_digest(f, "sha256")
        f.seek(0)
        embed_pack["jar_digest"] = base64.b64encode(digest.digest()).decode()
        embed_pack["jar_data"] = base64.b64encode(f.read()).decode()
    with open(android_library, "rb") as f:
        digest = hashlib.file_digest(f, "sha256")
        f.seek(0)
        embed_pack["lib_digest"] = base64.b64encode(digest.digest()).decode()
        embed_pack["lib_data"] = base64.b64encode(f.read()).decode()
    embed_pack_json = json.dumps(embed_pack)
    del embed_pack
    print(f"payload 打包完成: {len(embed_pack_json)} 字节")
    embed_bin(android_binary, embed_pack_json.encode())
    shipped = ship(android_binary, target)
    shipped_size = shipped.stat().st_size
    print(f"构建完成: {shipped}: {shipped_size} 字节")
    return shipped


def build_android_armv8() -> None:
    print("正在执行: build_android_armv8() ...")
    sdk_platform = "35"
    abi = "arm64-v8a"
    build_android(sdk_platform, abi)


def build_linux_binary(abi: str):
    rust_target = _abi_to_linux_target(abi)
    rustup_target_install(rust_target)
    env = os.environ
    try:
        _r = subprocess.run(
            [
                "cargo",
                "build",
                "--target",
                rust_target,
                "--bin",
                APP_NAME,
                "--release",
                "--no-default-features",
                # "--features",
                # "android",
            ],
            check=True,
            env=env,
        )
    except subprocess.CalledProcessError as e:
        raise RuntimeError(f"构建程序 {APP_NAME} 失败: {e}")
    res = (SCRIPT_DIR / "target" / rust_target / "release" / APP_NAME).resolve()
    if not res.is_file():
        raise FileNotFoundError(f"找不到构建产物 {res}")
    r = res.move_into(mktempdir())
    return r


def build_linux(abi: str) -> Path:
    target = _abi_to_linux_target(abi)

    linux_binary = build_linux_binary(abi)
    shipped = ship(linux_binary, target)
    shipped_size = shipped.stat().st_size
    print(f"构建完成: {shipped}: {shipped_size} 字节")
    return shipped


def build_linux_amd64() -> None:
    print("正在执行: build_linux_amd64() ...")
    abi = "x86_64"
    build_linux(abi)


# ---


def exit_not_implemented(detail: str) -> None:
    print(f"错误: {detail} 还没有实现。", file=sys.stderr)
    sys.exit(1)


def main() -> None:
    parser = argparse.ArgumentParser(description="构建脚本")

    platform_group = parser.add_mutually_exclusive_group(required=True)
    platform_group.add_argument(
        "--android",
        type=str,
        metavar="ARCH",
        help="构建 Android 平台",
    )
    platform_group.add_argument(
        "--linux",
        type=str,
        metavar="ARCH",
        help="构建 Linux 平台",
    )

    args, unknown_args = parser.parse_known_args()

    if unknown_args:
        exit_not_implemented(f"参数 {' '.join(unknown_args)}")

    if args.android is not None:
        if args.android == "armv8":
            build_android_armv8()
        else:
            exit_not_implemented(f"--android {args.android}")

    elif args.linux is not None:
        if args.linux == "amd64":
            build_linux_amd64()
        else:
            exit_not_implemented(f"--linux {args.linux}")


if __name__ == "__main__":
    init()
    main()
