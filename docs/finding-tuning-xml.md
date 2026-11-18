# 查找并配置本机 OEM 调音 XML

OpenDolby **不包含也不分发**任何厂商调音文件。要让内置扬声器校准正确工作，你需要从**这台笔记本自身**的 Windows 驱动分区（或厂商驱动包）中只读提取与本机硬件子系统 ID（Subsystem ID）严格匹配的调音 XML。

> [!CAUTION]
> **切勿跨机型混用调音文件**：不同机型的扬声器腔体谐振点、高通截止频率（HPF）与增益补偿差异很大。将其他机型的 XML 套用到本机内置扬声器可能导致破音甚至物理损坏。

---

## 1. 确认本机声卡的 PCI Subsystem ID

在 Linux 终端运行以下命令查看声卡控制器的子系统厂商与设备 ID：

```sh
lspci -nnvn | grep -A4 -i "audio\|multimedia"
```

或者直接读取 sysfs（无需安装额外工具）：

```sh
for card in /sys/class/sound/card*; do
  if [ -f "$card/device/subsystem_vendor" ]; then
    printf "%s: vendor=%s device=%s\n" \
      "$(basename "$card")" \
      "$(cat "$card/device/subsystem_vendor")" \
      "$(cat "$card/device/subsystem_device")"
  fi
done
```

例如输出 `vendor=0x1043 device=0x1693`（ASUS `1043`、机型子系统 `1693`），则对应的子系统标识通常写作 `SUBSYS_10431693` 或 `PCI_SUBSYS_16931043`。

---

## 2. 在本机 Windows 分区中只读查找 XML

如果这台机器保留了双系统 Windows 分区（假设已只读挂载在 `/mnt/windows`），可在 `DriverStore/FileRepository` 下按子系统 ID 搜索 XML：

```sh
find /mnt/windows/Windows/System32/DriverStore/FileRepository \
  -iname "*10431693*.xml" -o -iname "*16931043*.xml" 2>/dev/null
```

> 将上面命令中的 `10431693` / `16931043` 替换为你自己在第 1 步查到的四位厂商码 + 四位设备码组合。常见文件命名格式类似：
> - `DEV_0294_SUBSYS_10431693_PCI_SUBSYS_16931043.xml`
> - 或位于 `dax3_ext_*` / `dolbyaposwc_*` 驱动目录内。

如果已格式化 Windows 分区，也可前往笔记本厂商官网支持页下载**对应你精确机型**的 Windows 音频 / 音效扩展驱动包（`.exe` 或 `.cab`/`.zip`），使用 `cabextract` 或 `7z` 在本地解压后按同样的 Subsystem ID 查找。

---

## 3. 放置并验证调音文件

将找到的本机 XML 复制到用户配置目录（默认读取位置）：

```sh
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/opendolby"
cp /path/to/your_machine_tuning.xml "${XDG_CONFIG_HOME:-$HOME/.config}/opendolby/tuning.xml"
chmod 600 "${XDG_CONFIG_HOME:-$HOME/.config}/opendolby/tuning.xml"
```

使用 CLI 验证文件可正常解析且包含 `internal_speaker` 配置：

```sh
./target/release/dolby-rs inspect --profile dynamic
```

若输出了 `XML version`、可用预设列表（如 `dynamic`、`movie`、`music`、`game`、`voice` 等）及 `CP` / `VLLDP` 参数，即可打开 `dolby-gui`（或在 **设置 -> 调音文件** 中选择该文件）启用音效。
