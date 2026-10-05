---
name: GameLift
description: 两台 Windows 机器之间的游戏与文件夹搬运工具，界面是一块机房配线架
colors:
  lamp-flow: "#4c9dff"
  lamp-ready: "#35d39a"
  lamp-idle: "#e0a63c"
  lamp-fault: "#e2604f"
  panel: "#12161c"
  panel-rail: "#0d1116"
  panel-face: "#171d25"
  panel-edge: "#232b36"
  panel-hole: "#0a0d11"
  ink: "#e8edf5"
  ink-dim: "#93a1b4"
  ink-faint: "#7a8594"
  panel-rim: "#7a8594"
typography:
  port-name:
    fontFamily: "Segoe UI, Microsoft YaHei, system-ui, sans-serif"
    fontSize: "19px"
    fontWeight: 600
    lineHeight: 1.2
    letterSpacing: "-0.01em"
  title:
    fontFamily: "Segoe UI, Microsoft YaHei, system-ui, sans-serif"
    fontSize: "15px"
    fontWeight: 600
    lineHeight: 1.4
  body:
    fontFamily: "Segoe UI, Microsoft YaHei, system-ui, sans-serif"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: 1.5
  reading:
    fontFamily: "Cascadia Mono, Consolas, ui-monospace, monospace"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.4
    letterSpacing: "0.02em"
    fontFeature: "tnum"
  label:
    fontFamily: "Cascadia Mono, Consolas, ui-monospace, monospace"
    fontSize: "11px"
    fontWeight: 400
    lineHeight: 1.2
    letterSpacing: "0.16em"
  readout:
    fontFamily: "Cascadia Mono, Consolas, ui-monospace, monospace"
    fontSize: "22px"
    fontWeight: 400
    lineHeight: 1.2
    letterSpacing: "0.02em"
    fontFeature: "tnum"
rounded:
  none: "0px"
  lamp: "9999px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "12px"
  lg: "16px"
  xl: "20px"
  xxl: "24px"
components:
  button-primary:
    backgroundColor: "{colors.lamp-flow}"
    textColor: "{colors.panel-hole}"
    rounded: "{rounded.none}"
    padding: "0 16px"
    height: "36px"
    typography: "{typography.body}"
  button-primary-hover:
    backgroundColor: "#6fb0ff"
    textColor: "{colors.panel-hole}"
  button-primary-focus:
    backgroundColor: "{colors.lamp-flow}"
    textColor: "{colors.panel-hole}"
  button-key:
    backgroundColor: "{colors.panel-face}"
    textColor: "{colors.ink}"
    rounded: "{rounded.none}"
    padding: "0 12px"
    height: "32px"
    typography: "{typography.body}"
  button-key-hover:
    backgroundColor: "{colors.panel-face}"
    textColor: "{colors.lamp-flow}"
  button-quiet:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.ink-dim}"
    rounded: "{rounded.none}"
    padding: "0 16px"
    height: "36px"
  input-field:
    backgroundColor: "{colors.panel-hole}"
    textColor: "{colors.ink}"
    rounded: "{rounded.none}"
    padding: "0 12px"
    height: "36px"
    typography: "{typography.body}"
  port-face:
    backgroundColor: "{colors.panel-face}"
    textColor: "{colors.ink}"
    rounded: "{rounded.none}"
    padding: "16px"
  rail-row:
    backgroundColor: "{colors.panel}"
    textColor: "{colors.ink}"
    rounded: "{rounded.none}"
    padding: "10px 12px"
    height: "44px"
    typography: "{typography.body}"
  status-strip:
    backgroundColor: "{colors.panel-rail}"
    textColor: "{colors.ink-dim}"
    rounded: "{rounded.none}"
    padding: "8px 20px"
    height: "44px"
  lamp:
    backgroundColor: "{colors.lamp-link}"
    rounded: "{rounded.lamp}"
    size: "10px"
---

# Design System: GameLift

## Overview

**Creative North Star: "THE RACK PANEL"**

界面是一块机房配线架：左边一个机位是本机，右边一个机位是对端，中间一根跳线就是链路。所有状态都落在孔位上的灯里，所有动作都是插拔，没有第二套隐喻。

表达让位于任务。这块面板上没有卡片、没有阴影、没有圆角，也没有装饰插图；深度来自机架、面板、孔位三层色阶与 1px 缝线，节奏来自端口之间的留白。一屏之内同时出现的灯不超过四颗，颜色本身就是状态：待命、链路就绪、正在搬运、出错。

界面只在必要的地方有物理感：按键是孔位上的一个实体方块，选中的行在面板上亮起一层薄蓝，跳线是 2px 的折线。文字说人话，读数与图形同源——剩余时间有多长，行内那一条就走多长。

**Key Characteristics:**
- 直角、无阴影、无卡片，深度靠色阶与缝线
- 四色灯承担全部状态，颜色有唯一含义
- 等宽表格数字承担所有读数，数字不会跳动
- 跳线只走 45° 与 90°，是两端之间唯一的可视连接
- 一个面板上只有一个主按键

## Colors

深色机架底上只有四种信号色，其余全部是中性色阶。前端令牌就是实现里的 CSS 变量：`panel` 对应 `--color-panel`，其余同理。

### Primary
- **流量蓝（#4c9dff）**：正在搬运与唯一的主操作。主按键、进度带、选中的行、聚焦环都只用它。一屏只允许出现一个主按键。

### Secondary
- **链路青绿（#35d39a）**：链路就绪与完成。端口灯接上后变绿，选中的选择孔填充绿色，队列行完成后的状态字用它。

### Tertiary
- **待命琥珀（#e0a63c）**：等待。空插槽的灯、传入请求的边框、底部状态条在等你做决定时用它。
- **故障朱（#e2604f）**：失败与拒绝。错误文字、故障灯用它，且同一屏不与其他信号色并排出现。

### Neutral
- **机架炭（#12161c）**：窗口底色，面板之间的空处。
- **导轨黑（#0d1116）**：顶部阶段页签与底部状态条的底，比机架更深一层。
- **面板钢（#171d25）**：机位面、面板行所在的面。
- **缝线灰（#232b36）**：所有 1px 边界，只做边界，不做装饰。
- **孔位黑（#0a0d11）**：输入孔、进度槽、选择孔的内部。
- **亮字（#e8edf5）**：正文与机位名。
- **柔字（#93a1b4）**：次级说明、读数、状态条文字。
- **淡字（#7a8594）**：标签、路径、占位文字。--color-ink-faint 最低只能降到这个亮度：再暗在机架炭上不足 4.5:1。
- **孔位边（#7a8594）**：输入孔、选择孔、有边框按键的静态边界，保证控件在静止状态也能被认出来；结构性分隔线仍用缝线灰。

### Named Rules
**The Four-Lamp Rule.** 灯只有四种颜色，一种颜色只表示一种状态；需要第五种状态时改文案，不加颜色。

**The One Switch Rule.** 一屏只有一个主按键，任何时候用户只需回答"下一步是什么"。

**The Seam Rule.** 边界一律是 1px 的缝线灰；禁止彩色左边框、粗线强调与发光描边。

## Typography

**Display Font:** Segoe UI（回退 Microsoft YaHei、system-ui、sans-serif）
**Body Font:** 同上
**Label/Mono Font:** Cascadia Mono（回退 Consolas、ui-monospace、monospace）

**Character:** 无衬线正文字体负责说话，等宽字体只负责读数和标签。两套字体分工不重叠：能读的句子用正文字体，需要对齐或被比较的值用等宽。

### Hierarchy
- **Readout**（400，22px，行高 1.2）：待发清单的总量，一屏最多一个。
- **Port name**（600，19px，行高 1.2，字距 -0.01em）：机位名，两台机器的名字。
- **Title**（600，15px，行高 1.4）：面板行里的内容名。
- **Body**（400，14px，行高 1.5）：说明句、按钮文字、输入值。正文行宽不超过 75ch。
- **Reading**（400，13px，行高 1.4，字距 0.02em，表格数字）：速率、剩余时间、已传字节、地址与端口。
- **Label**（400，11px，字距 0.16em，全大写）：机位标签、阶段页签、状态字，一律用等宽。

### Named Rules
**The Tabular Rule.** 所有数字都是等宽表格数字，变化时不允许左右跳动。

**The Two-Line Rule.** 面板行最多两行：名字一行，路径或平台一行；第三行说明放到底部状态条。

## Layout

一块面板铺满窗口，默认 1100×760，最小 900×620。

连接阶段是三栏：本机机位、跳线、对端机位，栏宽比 1.15 : 0.85 : 1.15，栏间距 24px；跳线在中间列横向铺满，两端贴住机位边框。机位是铺满面板高度的方块，控制件挂在方块下方。900px 以下退化成上下排布，跳线仍在两个机位之间。

内容与传输阶段是两栏，左栏是来源与条目，右栏是待发清单或队列，栏间距 24px。

顶部是阶段页签，页签用底边对齐，当前页签抬高一级底色；底部是一条贴地的状态条，最小高度 44px，只写此刻该做的一件事，并把唯一的主按键放在最右。

间距只用 4 / 8 / 12 / 16 / 20 / 24 六档：同类元素之间 8 到 12，不同组之间 20 到 24，标题与内容之间永远小于 24。

## Elevation & Depth

没有阴影，一处都没有。深度只用三层色阶表达：导轨黑是最深的活动条，机架炭是底色，面板钢是抬起来的面，孔位黑是凹进去的孔。层级靠底色深浅与 1px 缝线读出来，不靠投影。

浮层只允许出现在真正需要打断的地方：传入的传输请求用琥珀色边框的面板，不用模态对话框。

### Named Rules
**The Flat-By-Default Rule.** 任何元素在静止状态都没有阴影；需要区分层级时改底色，不加投影。

## Shapes

直角。所有方块、按键、输入孔、进度槽的圆角都是 0，没有一张卡片。

唯一允许的圆是状态灯：10px 直径的正圆。跳线是 2px 的折线，只走 45° 与 90°，两端贴住机位，中间起拱。

选择孔是 16px 的方块，选中时填充链路青绿并在其中画一个深色对勾。

## Components

### Buttons
- **Shape:** 直角方块（0px 圆角），没有边框的按键靠底色区分，有边框的按键用 1px 缝线
- **Primary:** 流量蓝底配孔位黑字（36px 高，左右内边距 16px）；每屏只有一个
- **Key:** 面板钢底配亮字，1px 缝线边（32px 高，左右 12px）；悬停时文字转流量蓝、边框转流量蓝
- **Quiet:** 无底色，只有柔字；悬停时文字转亮字
- **Hover / Focus:** 颜色过渡 100ms；聚焦环是 2px 流量蓝，偏移 2px
- **Disabled:** 不换颜色，整体降到 40% 不透明度

### Inputs / Fields
- **Style:** 孔位黑底、1px 缝线边、直角（36px 高，左右 12px）
- **Focus:** 边框转流量蓝，不加发光
- **Placeholder:** 淡字，用完整的例子而不是提示词

### Navigation
- **Style:** 顶部是阶段页签，等宽小标签，全大写
- **Default:** 淡字，无底色；不可达的页签降到 40%
- **Active:** 缝线边加机架炭底，抬高一档，底部与内容区相接
- **Hover:** 柔字

### Rail Row
- **Shape:** 直角，行高 44px，行间靠 1px 缝线分隔
- **Background:** 选中时在机架炭上叠一层 10% 的流量蓝
- **Content:** 选择孔、名字、路径、读数、状态字
- **States:** 等待用淡字，传输中用流量蓝，已完成用链路青绿，失败用故障朱

### Port
- **Shape:** 直角方块，1px 缝线边，铺满所在栏的高度
- **Background:** 已接上是面板钢；未接上是虚线缝线边且无底色
- **Content:** 机位名、地址与端口、下方的控制件，右上角一颗状态灯

### Lamp
- **Shape:** 10px 正圆，无描边、无光晕
- **Colors:** 待命琥珀、链路青绿、流量蓝、故障朱
- **Rule:** 见 The Four-Lamp Rule；灯旁边永远有文字说明，颜色不单独承担信息

### Patch Cord
- **Shape:** 2px 折线，只走 45° 与 90°，两端贴住机位边框
- **States:** 未接上是虚线缝线灰；接上后是流量蓝实线；出错是故障朱
- **Motion:** 状态变化时一次 140ms 的阶跃，不做循环动画

### Status Strip
- **Shape:** 贴地一条，导轨黑底，1px 缝线上边
- **Content:** 一颗灯、一句此刻该做的事，最右是唯一的主按键

## Do's and Don'ts

### Do:
- **Do** 把状态放进四色灯里，并让灯旁边永远有文字。
- **Do** 用等宽表格数字承担所有读数（13px，字距 0.02em）。
- **Do** 让进度带的长度与剩余时间同源，读数与图形一起变。
- **Do** 用 1px 缝线灰划边界，用底色深浅表达层级；交互控件自己的边界至少 3:1，用孔位边而不是缝线灰。
- **Do** 让接收方在同意之前看不到任何落盘结果，界面文案也要说清这一点。

### Don't:
- **Don't** 使用圆角、卡片、阴影或玻璃模糊。
- **Don't** 引入第五种信号色，也不要用颜色单独表达状态。
- **Don't** 为"游戏工具"的类别默认服务：不要封面墙、不要启动器式的横幅、不要装饰插图。
- **Don't** 堆术语，界面上不出现 SMB、ACL、子网掩码这类词。
- **Don't** 让状态被视觉遮蔽：速率、剩余时间与文件路径必须始终在场。
- **Don't** 在同一屏放两个主按键，也不要让主按键离开底部状态条或它的动作位置。
