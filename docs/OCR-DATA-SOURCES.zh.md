# 终端、BIOS OCR 数据来源筛选

核查日期：2026-09-18。面向现有 ocrs 小模型微调、Rust/RTen 本地推理。本文完成来源筛选；尚未建立完整训练集或开始训练。

## 判断与选择

当前主要缺口是目标界面的覆盖、逐字可靠的标签，以及独立真实测试集。H20 不是这一阶段的主要制约因素；训练尚未开始，因此不能断言数据是唯一瓶颈。

本次检索没有找到同时具备 BIOS/服务器控制台覆盖、完整 OCR 真值、明确可用于产品训练的现成公开数据集。推荐以本项目真实截图为锚点，批量生成带精确标签的终端和固件界面，混入少量 HierText 保持通用能力。不要用大量手机/网页截图代替缺失的 BIOS 数据。

## 已核实的公开来源

| 来源 | 数据和监督 | 本项目用途 | 许可与限制 |
| --- | --- | --- | --- |
| [HierText](https://github.com/google-research-datasets/hiertext) | 11,639 张自然场景/文档图片，约 120 万词；词、行、段落及文字标注。训练/验证/测试为 8,281 / 1,724 / 1,634 张 | 推荐作为少量通用回放数据；ocrs 上游已有读取支持。它不是 BIOS 数据 | CC BY-SA 4.0；保留来源和相应许可 |
| [CodeSCAN](https://zenodo.org/records/10939237) | 12,000 张 VS Code 截图；25 种等宽字体、24 种语言、多种主题；代码区词/行标注及字符网格 | 视觉上最贴近命令、路径、符号和高亮；适合作为研究候选 | 数据页明确仅限学术研究，元数据许可为 `other-nc`；不默认加入产品训练 |
| [Google Screen Annotation](https://github.com/google-research-datasets/screen_annotation) | 22,417 张 RICO 手机界面的标注；类型、位置、文字/图片描述；训练 15,743 张。自动生成后人工校正 | 可选的界面文本/结构补充，优先级低于真实终端；需筛出文字标签并适配框 | 标注 CC BY 4.0；截图另从 RICO 取得并核对图像来源条款 |
| [OmniACT](https://huggingface.co/datasets/Writer/omniact) | 9,799 个任务记录，含桌面/网页截图、操作脚本、可交互元素框和标签 | 更适合以后做操作/元素定位；任务数不等于独立截图数，也不是全屏逐字 OCR 真值 | 当前官方数据卡 MIT；本次读取当前原始 README，未沿用搜索结果中的旧版 `ocr` 字段 |
| [ScreenSpot-Pro](https://huggingface.co/datasets/likaixin/ScreenSpot-Pro) | 高分辨率专业软件截图及指令目标位置 | 保留作未来 GUI 定位评估；不是全屏 OCR 训练集 | 官方数据卡 MIT；不要把评测集混入训练后继续宣称独立评测 |

CodeSCAN 的 [论文](https://arxiv.org/html/2409.18556v1)说明其文字标注针对代码区，而非整张截图的所有文字；若用于检测训练，要裁代码区或把未标注区域设为忽略，不能当作无文字背景。发布文件 `codescan.zip` 为 7,859,577,883 字节，MD5 为 `6cc76d8865d37c1e381cd5eb741a57f6`。下载入口见上表，完整包本次未下载。

HierText 原始标注位于仓库 `gt/train.jsonl.gz`、`gt/validation.jsonl.gz`、`gt/test.jsonl.gz`，图像入口为：

```text
s3://open-images-dataset/ocr/train.tgz
s3://open-images-dataset/ocr/validation.tgz
s3://open-images-dataset/ocr/test.tgz
```

按官方说明可用 `aws s3 --no-sign-request cp` 下载。只从训练划分抽取回放样本；不要使用验证/测试标签训练。[ocrs 训练指南](https://github.com/robertknight/ocrs-models/blob/main/docs/training.md)已有此数据的接入流程。

## 更有价值的目标域数据生产来源

这些是制作数据的工具/素材，不是已经标注好的训练集。

| 来源 | 如何得到可靠标签 | 需要补足的部分 |
| --- | --- | --- |
| [xterm.js](https://github.com/xtermjs/xterm.js)，MIT | 输入受控 ANSI 流，渲染截图，同时从最终可见 buffer 导出字符、行列；[IBufferLine](https://xtermjs.org/docs/api/terminal/interfaces/ibufferline/)提供逐格/逐行读取 | 加入真实 Linux 虚拟控制台字体和低分辨率样例。按渲染后的可见内容标注，不能直接拿原始 ANSI 字节或滚屏前文本作真值 |
| [tldr-pages](https://github.com/tldr-pages/tldr)，文档 CC BY 4.0 | 提供命令与参数的文本素材，再配合自有随机 IP、端口、路径和日志模板渲染 | 它没有截图或命令输出；示例只作文本素材，不在生产机器上执行 |
| [SynthTIGER](https://github.com/clovaai/synthtiger)，代码 MIT | 可生成文字图、精确转录、字符坐标及掩码；定制等宽字体、颜色和界面主题 | 内附字体按各自许可，见 [NOTICE](https://github.com/clovaai/synthtiger/blob/master/NOTICE)。优先定制目标域合成，不优先下载其 1,000 万张通用场景文字包 |
| [EDK II / OVMF](https://github.com/tianocore/edk2/tree/master/OvmfPkg) | 在受控虚拟机中采集真实 UEFI 菜单、启动和 Shell 页面；可从渲染路径/字符串资源辅助生成标签 | 源码字符串并不等于当前可见文字，需记录最终布局、遮挡和截断。不能代表 Supermicro Aptio 的全部风格；[OvmfPkg 许可](https://github.com/tianocore/edk2/blob/master/OvmfPkg/License.txt)主要为 BSD-2-Clause-Patent，另有文件例外 |
| [联想 BIOS Simulator Center](https://support.lenovo.com/us/en/solutions/ht502745) | 官方提供超过 1,000 款产品的交互 BIOS 样式，可研究菜单、弹窗和选中状态 | 是界面参考/潜在采集来源；不是已授权训练集，尚未确认训练及再分发许可。覆盖机型主要为 PC/工作站，不能当作 1,000 种服务器固件 |

建议同时走两条生成路径：文字行合成覆盖字符与退化组合；完整终端/UEFI 渲染覆盖检测、遮挡和交互状态。最终准确性必须在真实 BMC 截图上验证。

这里的绘制和标注规则用于生产训练数据，模型推理时不需要硬编码 BIOS 坐标、颜色或菜单词典。离线生成工具可以使用浏览器；这不会给交付的 Rust iKVM 工具引入浏览器依赖。

## 第一轮具体数据配方

以下为试验预算，尚未生成，也不是已验证的最优比例。

1. **真实锚点：**现有 161 张截图按 RGB 像素去重为 74 张，先制作约 500–2,000 个精确转录裁片，保留完整截图、框、菜单/会话及同源分组。优先标注低对比度选中行、`0/O`、`1/I/l`、`[]`、`_`、数字、IP 和配置值。
2. **目标域合成：**先生成 100,000 个文字行，终端和 BIOS 风格各约一半；覆盖多种等宽/点阵字体、字号、黑底/浅底/蓝底、反色、禁用灰字、BMC 常见缩放与压缩。用随机内容避免背诵固定 BIOS 菜单。
3. **通用补充：**从 HierText 训练集筛选约 10,000 个可辨认、水平、非手写、符合当前字符集的文字行。采样时控制比例，不能让大数据集淹没少量真实 BIOS 样本。
4. **独立验收：**另采至少 100–200 张新真实截图作为第一轮窄场景验收预算，覆盖终端、BIOS、弹窗和选中状态；若范围仍只限定当前固件，按未参与训练的菜单/内容/采集会话分组。要宣称跨固件能力，必须加入未见固件和字体的真实测试图。

现有图片经过反复调试，不当作全新最终验收集。同一菜单的四台机器、同一源图的裁片和增强变体不能跨训练/验证/测试集合。先固定检测器微调识别模型，比较真实数据、真实加合成、再加通用回放三种组合；检测漏行和选中状态另行评估。

后续扩展预算仍参考 [垂直微调评估](OCR-FINETUNE-EVALUATION.zh.md)：5,000–20,000 个真实行、500–1,500 张检测标注全屏。选中/禁用/弹窗和标签值配对还需独立状态监督；不能认为文字数据自动提供这些标签。

## 本次实际核查与落盘

- 已读取官方论文、数据卡、下载元数据和许可说明；已下载少量说明及 Google Screen Annotation 训练集前 5 条标注。
- 该 CSV 的实际列名是 `screen_id`、`screen_annotation`，与 README 的概念字段名 `image_id`、`label` 不同。标签包含 UI 类型、位置和文字，不能把全部内容直接作为文字转录。
- 核查文件位于 `artifacts/ocr-data-research/`，下载来源与 SHA-256 记录在 `download-manifest.json`；样本位于 `screen-annotation-train-sample.json`。此目录已被项目原有 `/artifacts/` 规则忽略。
- 尚未下载完整图片集、审核配对截图质量、批量标注或训练；已核实的来源不能等同于已完成训练数据验收。
- 本次只作公开来源研究与本地文件整理，没有连接、重启或更改服务器。
