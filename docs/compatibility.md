# 硬件兼容性与设备支持说明

OpenDolby 当前仍处于实验阶段。**仅在明确识别为本机内置扬声器（且处于可用状态）时**，才会自动套用本机 OEM 调音 XML，不会对任意声卡、外接耳机、蓝牙或 HDMI 设备通用套用。

## 已实测机型

| 品牌 / 机型 | 音频编解码器 / Subsystem | 发行版 | 状态 | 说明 |
| --- | --- | --- | --- | --- |
| ASUS Zenbook UX3404VC | Realtek ALC294 (`0x10431693`) | Fedora (PipeWire) | 已实测通过 | 实机验证主路径、A/B 电平匹配、后台单实例与路由恢复 |

## 支持边界说明

- **其他机型**：解析逻辑支持读取标准 OEM 调音 XML（本机查找方式见 [finding-tuning-xml.md](finding-tuning-xml.md)），但需要对应机型的本机 XML 与实机测试验证，**不代表已开箱支持**。
- **非内置扬声器**：耳机、USB 声卡、蓝牙音频、HDMI/DP 输出与虚拟 Sink 均不会自动套用扬声器 OEM 调音。
- **跨平台**：仅支持 Linux + PipeWire，**不支持** macOS 或 Windows。

## 社区借鉴与边界

- 设备发现与按设备绑定的思路参考了社区项目 `speaker-tuning-to-easyeffects` 与 EasyEffects 的按设备配置理念。
- 本项目仅借鉴工程思路，**未复制任何 GPL 代码**，也**不继承**上游项目已列出的数十款机型支持列表。

## 反馈与测试清单

如果你在其他 Linux 机型或发行版上测试 OpenDolby，欢迎在反馈时提供以下脱敏信息（**请勿上传 OEM 原始文件或包含序列号的完整系统 dump**）：

- **机型型号**：例如 `ASUS Zenbook UX3404VC`
- **发行版**：例如 `Fedora 41` / `Ubuntu 24.04`
- **PipeWire 版本**：可通过 `pipewire --version` 获取
- **设备类型**：内置扬声器 / 耳机 / USB / 蓝牙 / HDMI
- **脱敏设备节点**：`pactl` 或 `wpctl` 中对应的节点名称与描述（移除序列号等敏感字段）
- **XML 来源**：本机提取路径或来源说明（无需附带文件本身）
