# clean_c · 长期项目笔记

基于 egui/eframe 的**跨平台**磁盘清理工具（Windows / macOS / Linux）。
核心诉求是**安全清理**：白名单 + 默认拒绝，宁可不删也不误删。

## 目录与模块
`src/{main, app, model, targets/*, scanner, cleaner, recycle, safety, settings, sysutil, theme, advice, bigfiles, apps, ui/*}`

## 跨平台架构约定（重要）
- **`sysutil` 是唯一直接接触操作系统的模块**。其它模块一律通过它拿路径、容量、
  权限、打开文件管理器。要接新平台能力就先在这里加抽象函数，不要在业务代码里
  写 `#[cfg]`。
- **`targets/` 按平台拆文件**（`windows.rs` / `macos.rs` / `linux.rs`），
  `targets::build()` 按 `cfg` 分发。**这是唯一允许删除文件的地方**，
  加清理项只改对应平台的文件。
- `safety` / `cleaner` / `scanner` / `bigfiles` **不写 `cfg` 分支**，只依赖
  `safety::SEP` 与 `sysutil` 抽象。
- 跨平台"当前平台用不到"的项（枚举变体、常量、函数）统一加
  `#[allow(dead_code)]` **并在注释里写明在哪个平台用**，保证三平台 `cargo check`
  零告警。
- 路径规范化：`safety::norm` —— Windows 反斜杠+小写，macOS 斜杠+小写，
  **Linux 斜杠且保留大小写**（`\` 在 Linux 是合法文件名字符，不能替换）。
- 目录树前缀匹配统一用 `safety::is_within` + `safety::SEP`，不要手写
  `starts_with(base + "\\")`。
- **Windows 静态保护名单写的是"相对系统盘根目录"的片段**（`\windows\system32`），
  使用时必须补真实盘符（`safety::system_drive()` 从 `%SystemRoot%` 反推，
  系统盘不一定是 `C:`）。
- **千万不要把 `C:\Windows` 整棵拉黑** —— `Temp` / `Logs` / `Prefetch` /
  `SoftwareDistribution\Download` / `Minidump` 都是清理目标，拉黑后一个都删不掉。
  同理 macOS 不能拉黑整个 `~/Library`（`Caches`/`Logs` 要清理），
  Linux 不能拉黑 `~/.local`（回收站在里面）。

## 安全基线（改任何代码都不能放松）
1. 只允许白名单根目录内的路径；2. 不跟随符号链接；3. 路径不许含 `..`；
4. 白名单根目录本身保留，只清内容；5. unix 关键文件保护（`passwd`/`shadow`/
`sudoers`/`authorized_keys`/`id_rsa` …）。

## 构建与验证矩阵（每次改完都跑）
```
cargo fmt
cargo check --all-targets                        # Windows
cargo check --target x86_64-unknown-linux-gnu --all-targets
cargo check --target aarch64-apple-darwin --all-targets
cargo test
cargo build --release
./target/release/clean_c.exe --selftest          # 安全抽样必须全 ok
./target/release/clean_c.exe --dryrun            # 「安全拦截」必须为 0！
./target/release/clean_c.exe --test-trash        # 单文件 + 批量都要成功
```
**`--dryrun` 的安全拦截不为 0 就说明保护名单和白名单打架了**（第六批踩过：
`C:\Windows` 被整棵拉黑）。

- Linux 交叉编译需要 `eframe` 的 `x11`/`wayland` 特性（`Cargo.toml` 里有
  专门的 `[target.'cfg(target_os="linux")'.dependencies]` 段）。少了它 winit 直接
  `compile_error!`。
- **Windows 上 `cargo check` 过不代表 Linux 过**：`#[cfg(unix)]` 的代码在
  Windows 上一次都不会被编译。删函数时一定要交叉检查。

## GUI 截图验证（项目习惯）
用纯 stdlib 的 Python（ctypes 调 GDI `BitBlt`/`GetDIBits` + zlib 手写 PNG）截图，
脚本在 `target/shot.py`（target 不入版本控制，删了要重建）：
- 用 `GetClientRect` + `ClientToScreen` 取**客户区**坐标，截图坐标 = 点击坐标
  （用 `GetWindowRect` 会差一个标题栏高度，会点错导航项）。
- **抓图一律用 `PrintWindow(hwnd, memDC, 2)`（`PW_RENDERFULLCONTENT`），不要用屏幕 DC
  的 `BitBlt`**。屏幕 DC 在窗口被遮挡时会把上层窗口一起拍进来 —— 2026-10-16 就因此
  在四张截图的同一位置看到一段"caution 色小条 + 一行小字"，白白查了很久，换
  PrintWindow 后那半截立刻变成程序自己的顶栏标题。PrintWindow 只渲染该窗口自身，
  连抓三帧逐像素完全一致。脚本 `/tmp/shot2.py`（`EnumWindows` 找 hwnd + `GetClientRect`
  + zlib 手写 PNG）。注意它的 y 比屏幕 DC 低 1px，跨工具比坐标要留这 1px。
- 窗口标题是 `磁盘清理 · CleanC`；客户区 1120×760。
- **应用每次重启都默认停在「概览」页**，不会记住上次的页。要拍别的页先点侧栏。
- 侧栏导航的客户区**点击** y 坐标（**2026-10-16 实测**，条目高 34、条目间距 44px、
  组标题 11px 字号）：概览 107 / 专项清理(组标题，点了不翻页) 156 / 智能清理 193 /
  大文件 237 / 应用(组标题) 282 / 软件卸载 323 / 系统(组标题) 641 / 设置 677 /
  安全说明 721；点 x 用 120 最稳。**别靠"比底色暗"扫行带**（只扫出 3 条，漏掉一半），
  用模板匹配：`/tmp/ncc.py <png> "0,40,212,760" 概览 智能清理 大文件 …`，NCC 0.7+ 即命中。
  「系统」这一组是**贴底排的页脚**，改侧栏的间距就会让这 3 个数整体平移 ——
  动过 `widgets.rs` 里的 `NAV_*` 常量之后一定要重新量一遍，别沿用旧坐标。
- **测边缘/抗锯齿之前先确认右侧领空干净**：`ScrollStyle::floating()` 的滚动条不占
  布局宽度，会浮在内容最右边的像素上，把亚像素测量的残差从 0.04px 抬到 0.38px。
  要么等它淡出（静置抓图），要么先看 x1096..1109 有没有灰条。
- **每次动作前都要重新置顶**（`bring_front()`）：微信这类窗口自己会抢最前，
  应用重绘时 winit 也会重排 z 序。抓完还要用 `looks_like_app()` 抽样侧栏像素自检，
  不像本程序就重试，否则会"成功保存"一张聊天窗口的截图。
- **首次点击常被吞**（别的进程 `SetForegroundWindow` 会被前台锁定拒绝，第一次点击
  只起激活作用）。截图脚本里先来一次无害点击热身即可 —— 这是工具问题，不是应用 bug。
- 用 Bash 的 `run_in_background: true` 启动 GUI 可在后续命令中存活。

## 自绘窗口外壳（无边框）
`main.rs` 里 `with_decorations(false)`，系统标题栏与最小化/最大化/关闭全部关掉，
改由 `ui::chrome` 自绘：
- `title_bar()`：三个 **26×26** 按钮（10px 图元、右边距 9、`Stroke` 1px）+ 拖拽区
  （`ViewportCommand::StartDrag`）+ 双击切换最大化。按钮组用
  `Layout::right_to_left` 整体右对齐（**别手算剩余宽度**，算错过）；
  `item_spacing.x` 必须显式置 0，否则手算的横向布局会被默认 8px 间距顶到屏幕外。
  **换页时按钮位置必须逐像素不变**（用户明确要求）：实测概览/智能清理/大文件/
  软件卸载/设置五页的图元 bbox 都是 最小化 x1037..1046 / 最大化 x1064..1075 /
  关闭 x1093..1102。
- `resize_handles()`：7 个不可见 `Area` + `BeginResize(ResizeDirection)`，
  否则无边框窗口就再也拖不动边。
- 面板声明顺序 = 布局顺序：标题栏 → 侧栏（通高）→ 状态栏 → 顶栏 → CentralPanel。

## 侧栏（`chrome::side_nav`）
- 分组写在 `NAV_GROUPS`（`("", …)` = 不加标题的「概览」）；画一组的活统一走
  `nav_group_block`，**留白只有一份定义**（`widgets::NAV_GROUP_TOP` / `_UNDER` /
  `NAV_ITEM_GAP`）。
- **最后一组（「系统」）是贴底页脚**，用嵌套的 `Panel::bottom("cleanc_nav_footer")`
  实现：面板高度按内容自适应，而且会先把空间从父级扣掉，上面几组再往下排 ——
  **不要再回去手算「可用高度 − 组高」**。
- ⚠️ **egui 在每个 `allocate_*` 之后都会再加一个 `spacing.item_spacing`**，
  而 `add_space` 又会把 `min_rect` 撑到光标位置。本程序全局把 `item_spacing`
  设成 `(8,8)`（`theme.rs`），所以：
  - 两条相邻导航实际间距 = `8 + NAV_ITEM_GAP`，**不是** `NAV_ITEM_GAP`；
  - 一组的高度 = 标题高 + 各条目高 + 各 `add_space` + `(标题数 + 条目数) × 8`
    **还多一个尾随的 8** —— 上一版正是漏算这些，「安全说明」溢出侧栏
    下边距 14px、连窗口底都越过 2px（用户报的就是这个）。
  - 结论：**能交给 egui 算的尺寸就别手算**。
- 页脚顶上有一条 `widgets::nav_separator`（两端内缩到图标列 `NAV_ICON_LEFT`），
  用 `p.nav_line`：侧栏底本来就深，拿 `p.border` 画上去只有 Δ13，等于看不见，
  所以单独开了一档色（浅 `#CDD8E7` Δ40 / 深 `#242C37` Δ83，与主区
  `border` 在各主题下的可见度相当）。它的作用是给「系统」上方那一大段空白一个交代。
- **别把分组留白叠两次**：页脚里再 `add_space` 一次就会比别处厚一倍（踩过，
  实测多出 12px），`nav_group_block` 里已经给过了。

## 画渐变/圆角胶囊的两条硬规则（都踩过）
1. **沿轮廓按角度采样，别按矩形均匀切条**：均匀横切会让圆角处切出阶梯。
   用 `theme::gradient_rows`（`y = top + r(1−cosφ)`）。
2. **egui 的 `Mesh` 是硬边光栅化**（像素中心落在轮廓里就涂满，最多多盖 0.5px），
   而羽化环只能**往外**补覆盖率、减不掉已涂满的像素。所以：
   填充按 `FILL_INSET = 0.5` 内缩让硬边接上环带不透明的内边，环带**骑在轮廓上**
   （内 0.5 不透明、外 0.5 透明）。
   **内缩到 1.0 会留半像素缝**（残差 0.039 → 0.407px）。
   量化验证用亚像素圆拟合（`edge = x0 + Σcoverage` 后最小二乘），
   名义半径 26 的胶囊应拟合出 RMS < 0.05px。

## 中央内容区的滚动条过道
`ScrollStyle::floating()` 的 `floating_allocated_width` 默认 **0.0** —— 滚动条不占
布局宽度，直接浮在内容最右边，会压在「一键清理」按钮的圆角上（静止时它淡出，
**一滚动就露出来**）。修法：`ui.style_mut().spacing.scroll.floating_allocated_width
= 12.0`，同时把 CentralPanel 内边距右侧从 22 减到 10，让内容右边界保持不变。
**不要用"加大 frame 内边距"来留过道** —— 那会把 ScrollArea 一起往里挪，
内容照样贴着滚动条。该字段是静态量（只看"是否需要滚动"），不会让内容抖动。

## 看设计稿的方法
拿到设计图先**逐像素量颜色**，别靠眼睛猜。`PIL` 装在托管 venv
`C:\Users\Administrator\.workbuddy\binaries\python\envs\default`（**这个 venv 是
2026-10-16 为解码 webp 才建的**，托管 python 本体没有 PIL）。
量完再和现有 `Palette` 对照 —— 经常发现"其实已经很像了，只有一两个字段偏了"，
比整体重画稳妥得多。

**几何尺寸要按"内容宽比例"统一换算，不能挑着换。** 参考图是先按它自己的窗口宽度
排的；本程序内容区更宽（有 212px 侧栏），照抄参考图的原始像素值会让元素显得偏小。
智能清理页的算法：参考图内容宽 722（x100..822），本程序 873（x234..1107），
比例 1.209 —— 大圆、卡片高宽、间隙、内边距、勾选框**全部乘这个系数**。
（2026-10-16 踩过：只把大标题和进度条宽放大了、大圆照抄 108，
结果大圆占内容宽 12.4%、参考图是 15.0%，肉眼就是偏小。）
**逐像素量边界要挑没有文字的列/行**：卡片高度就是在一张卡的内部逐行取色，
看到 y384 起整行回到页面底色才定的 —— 之前看着 y408 那里有东西就当成卡底，量错了 24px。

## 产品文案约定
- 全局叫「磁盘清理」（不是「C 盘清理」）；磁盘名用 `sysutil::mount_label`，
  不要在界面里硬编码 `C 盘`。
- 回收站/废纸篓的说法统一走 `recycle::trash_name()` /
  `recycle::action_label()`（macOS 是「废纸篓」「移入废纸篓」）。
- 权限的说法走 `sysutil::privilege_label()`（Windows「管理员」/ unix「root」）。
- 侧栏只负责导航，不放版本号之类的说明文字；那些信息在「设置 → 关于」卡片里。
- CLI 表格用 `main.rs::pad_display` 按**显示宽度**对齐（汉字占 2 列），
  别用 `{:<26}`。

## 其他踩坑
- egui 0.36 `Slider` 轨道画的是 `widgets.inactive.bg_fill`（不是 `extreme_bg_color`）。
- `egui::Modal` 内容超高会被裁掉，弹窗一律用 `dialogs.rs` 的三段式骨架
  （`modal_begin` / `modal_scroll` / 底部固定）。
- 回收站通道**不能只看返回码**：本机 `SHFileOperationW` 会返回虚假错误码 2，
  判据是"调用后文件是否还存在"。
