# Live2D 接口与自定义设计（v1）

公开的是 Aster 的控制代码、JSON 配置格式、参数映射、动作/表情示例和类型定义。**弄玉的模型、贴图、原始动作文件及第三方 SDK 仍在本地，不随仓库分发。** 不需要改 Rust 或 JavaScript 就能设计新的表情和关键帧动作。

## 先试一个自定义配置

从仓库目录运行：

```sh
aster --check-companion examples/companion-custom.json
aster --companion-profile examples/companion-custom.json
```

在 TUI 中：

```text
/emotion focused 0.7
/motion acknowledge 0.8
/look
/pet reset
/pet info
```

`/emotion`（别名 `/mood`）和 `/motion` 不带参数时列出当前配置里的名字。strength 为 0–1，默认 1。表情持续到下次切换或重置；动作播放一次，新的动作替换当前动作。`/pet reset` 清除表情、动作和定时视线，保留真实工作状态。`/pet info` 显示参数、兼容性提示、当前控制状态及最近一次渲染器确认。

这些操作只控制形象，不触发模型请求、执行命令、批准文件写入，也不能把失败的检查改为通过。工作状态仍来自 harness。默认的 `happy`、`heart`、`angry` 是配置名称，不代表现实人物的心理状态。

## 给 AI/autodesign 的配置契约

- [profile.schema.json](../live2d/profile.schema.json)：编辑器/生成器可读的 JSON Schema。
- [companion.d.ts](../live2d/companion.d.ts)：JavaScript 接口的 TypeScript 定义。
- [弄玉默认配置](../live2d/profiles/nongyu.json)：原有参数映射、4 个 emotion，以及 `nod`、`shake`、`tilt`。
- [自定义示例](../examples/companion-custom.json)：增加 `focused` emotion 和 `acknowledge` motion。
- [通用模型起点](../examples/companion-generic.json)：标准参数、全身取景；修改 model 路径并根据自己的 rig 调整。

配置中的 `model` 相对于 `--pet-dir`，例如 `my-model/model.model3.json`。该目录还需要本地 `vendor/` SDK，布局见 README。`--companion-profile FILE` 或 `ASTER_COMPANION_PROFILE` 显式选择配置；项目里的任意 JSON 不会自动执行或加载。重新启动 Aster 后生效。

```json
{
  "duration_ms": 1500,
  "blend": "add",
  "tracks": {
    "head_y": [[0, 0], [0.25, 5], [0.5, -2], [1, 0]],
    "head_z": [[0, 0], [0.5, -3], [1, 0]]
  }
}
```

这是一个 `motions.NAME` 值。每个 track 使用绑定的语义通道，关键帧格式为 `[归一化时间, 参数值]`，时间从 0 到 1 严格递增，线性插值。`add` 将值加到当前姿态；`set` 向绝对值混合。强度和首尾 100 ms 混合权重共同生效。动作结束后恢复状态驱动的姿态，不循环。这里是 Aster 自己的关键帧格式，不是 Cubism `.motion3.json` 格式。

emotion 是 `通道名 → 目标值`，例如 `"focused": {"eye_y": -0.2, "smile_l": 0.15, "smile_r": 0.15}`。`neutral` 必须存在，可以为空。每帧从 rig 默认值及工作姿态开始，再应用 neutral、所选 emotion、定时视线和动作。未在当前 emotion 中指定的通道不会保留上一表情的值。常用通道有 `head_x/y/z`、`body_x`、`eye_x/y`、`eye_l/r`、`breath`、`mouth_open`、`smile_l/r`；还可以定义模型特有通道。

实际写入值限制在 **当前模型自身的 min/max** 中。缺失的参数会显示在 warnings 里；完全没有可用参数的动作/非空表情会被拒绝，部分支持时回执列出 `supported_channels`。参数名称存在不保证视觉效果理想：尤其眉毛、嘴形、配饰等非标准变形需要人工预览。默认弄玉配置不驱动其不对称眉毛/嘴形变形。

布局：`zoom` 是适配画布后的倍率；`center: [x,y]` 是画布中的目标位置，`anchor: [x,y]` 是模型边界内的对齐点，坐标都是 0–1。通用配置默认全身，弄玉配置保留原有半身取景。

限制：配置 128 KB；1–128 个唯一参数绑定；最多 32 个表情、32 个动作；动作 100–10000 ms，每条 track 2–64 帧；名称为至多 64 位 ASCII 字母/数字/下划线/连字符。值必须有限且在 ±10000 内。配置、控制消息拒绝未知字段；路径只能是本地相对路径。JSON 不包含可执行代码。运行时校验还检查跨字段引用、重复参数和关键帧顺序，不能只依赖 Schema。

## 导出实际能力，生成后再验证

```sh
aster --companion-profile examples/companion-custom.json \
  --live2d-probe .aster/qa/my-design \
  --probe-emotion focused --probe-motion acknowledge
```

这会启动 Aster 自己的私有渲染进程，不打开网站或调用模型，退出时清理进程。输出包括：

- `interface.json`：实际参数 ID、默认值、min/max、可用动作/emotion、缺失绑定。
- `controls.json`：emotion（强度 0.7）、motion（强度 0.8）、look、reset 的真实回执和最终写入参数。
- `idle.png`、`speaking.png`、`neutral.png`、`emotion.png`、`motion.png`、`look.png`、`reset.png`：本地预览帧。
- `renderer.json`：加载和动画诊断。

仅使用 `--live2d-probe DIR` 也会导出接口清单。`--probe-emotion`/`--probe-motion` 需要同时指定它。预览帧是回执时刻的采样，不代表完整动画；终端约 8 fps，短动作可能被跳过部分关键帧。

建议 autodesign 流程：读取当前 `interface.json` 和 Schema → 从示例复制完整配置 → 只修改绑定、emotion、motion 或构图 → 运行 `--check-companion` → 用上述探针检查参数及预览 → 在 TUI 中连续播放，确认效果后再使用。`--check-companion` 无需模型资产，只验证配置；探针才验证实际 rig。

可交给生成器的提示：

> 根据这个 interface.json、profile.schema.json 和现有配置，设计一个克制的点头回应动作和专注表情。只使用实际存在的参数 ID，保持模型路径不变，避免眉毛/嘴形的未经验证变形。输出完整 JSON，不含代码。动作 1–2 秒，关键帧从 0 到 1 严格递增。先通过 Aster 校验，再检查探针参数及预览，不要声称仅凭 JSON 就验证了视觉效果。

## JavaScript / Rust 接口

`live2d/companion.js` 是不依赖 DOM 的原始控制模块，浏览器暴露 `AsterCompanion`，Node 支持 `require()`。在现有 Cubism4/Pixi 集成里：

```js
const controller = AsterCompanion.create(model.internalModel.coreModel, profile);
controller.describe();
controller.state('reading');
controller.emotion('focused', 0.7);
controller.motion('acknowledge', 0.8);
controller.look(0.4, 0.2, 1500); // x/y: -1..1; positive x/right, y/up
// 每帧一次；在 physics 更新后、绘制前运行。
model.internalModel.on('beforeModelUpdate', () => controller.apply(deltaMs));
controller.snapshot();
controller.reset();
```

输入的 `deltaMs` 必须为 0–250。`describe()`/`snapshot()` 返回独立副本。Aster 已注册更新回调，**不要在 Aster 内重复 apply**；内置实例为 `window.asterCompanion`。

JSON 消息入口为 `controller.dispatch(command)`：

```json
{"type":"emotion","name":"happy","strength":0.7}
{"type":"motion","name":"nod","strength":0.8}
{"type":"look","x":0.4,"y":0.2,"duration_ms":1500}
{"type":"reset"}
```

每行是独立消息。直接 JavaScript 调用校验失败时抛错。Rust 入口为 `aster::live2d::Companion::control(aster::companion::Control)`，返回请求 ID；`current().info["control_results"]` 包含相同 ID 的 `ok/result` 或 `error`。队列最多 32 条，发送成功只表示入队，**渲染器回执才表示接受**。`current().info["control"]` 保存最新状态和实际写入参数。回执保留最近一批，不是持久消息总线。

这是可嵌入的模块接口和 Rust 桥接接口，**没有公开 HTTP 控制服务**。现有本地资源服务仍使用随机路径、Host 检查和文件白名单。模型中引用的 textures/physics/pose/motions/expressions 可本地加载；本版命名控制采用上面的参数动作/表情格式，不提供直接播放任意原生动作或音频的命令。

上游集成依据：[Pixi Live2DModel](https://guansss.github.io/pixi-live2d-display/api/classes/index.Live2DModel.html)、[Cubism4 更新顺序](https://github.com/guansss/pixi-live2d-display/blob/master/src/cubism4/Cubism4InternalModel.ts)。本仓库的接口代码不替代相应 SDK/模型的许可。
