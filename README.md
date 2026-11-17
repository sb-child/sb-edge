# sb-edge

色妹妹自用的边缘网络服务端。我上次做这种项目还是在原生家的时候实现手机遥控院子里的电动门。

todo:

- [x] 构建脚本
- [x] java一堆破事
- [x] 嵌入和取出 payload
- [ ] 把主函数抄过来
- [ ] entrypoint

## 构建

为了方便，我手写了没有第三方库依赖的 `build.py` 内置了一堆中间步骤。

- 为 amd64 Linux 设备构建: `python3 build.py --linux amd64`
- 为你的手机(通常是armv8)构建: `python3 build.py --android armv8`

报错通常意味着你没有安装 rust，某个 crate，Android NDK 之类。报错应该足够让你知道怎么修。

## 运行

**Linux**: 直接运行就可以了。

调用链大概长这样：

```text
sb-edge(bin) 参数解析
 \- entrypoint()
```

**Android**: 在 ADB 或者 Termux 运行，如果你给终端装了 Shizuku 的 rish，它会自动调用。

调用链大概长这样：

```text
sb-edge(bin) 参数解析，释放 dex 和 lib，看看有没有 rish，然后 spawn
 \- rish那一坨(可选)
    \- app_process -> com.sbchild.sb_edge -> launchSbEdgeAndroid() -> JNI
       \- sb-edge(lib) -> 处理java一堆事
          \- entrypoint()
```

## License

MIT
