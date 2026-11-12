#!/usr/bin/env python3
"""Generates the Chinese layout test documents under testdata/zh.

Each document follows the conventions of a real kind of Chinese document
(official notice, contract, regulation, thesis, report) or isolates one family
of features (type sizes, character effects, indents and spacing, number
formats, punctuation). Paragraphs say in their own text which formatting they
carry, so a rendering can be judged by reading it.

The documents are written by hand-built markup, not saved by Word. They
exercise the mechanisms Chinese documents rely on; they are not a substitute
for real documents saved by Word.

    python3 tools/zh/gen_corpus.py
"""

from pathlib import Path

from docx_builder import (
    SIZES, abstract_num, default_styles, field, level, num, numbering, p, page_break, ppr, r, rpr, section,
    start_override, style, write,
)

OUT = Path(__file__).resolve().parent.parent.parent / "testdata/zh"

TEXT_A = (
    "文档排版的目标是让读者把注意力放在内容上。合适的字号、行距和页边距能够减轻阅读负担，"
    "而前后一致的标题层级和编号方式则帮助读者快速建立全文的结构。排版规则并不复杂，"
    "难的是在每一页、每一段都严格执行。"
)
TEXT_B = (
    "在浏览器中直接显示办公文档，需要解决三个问题：第一，准确读出文件中记录的格式；"
    "第二，按照与原软件相同的规则计算每一行的位置；第三，在没有原字体的情况下保持换行和分页不变。"
    "任何一步出现偏差，页数就可能对不上。"
)
TEXT_C = (
    "OpencloudOffice 的内核使用 Rust 编写，编译为 WebAssembly 后体积约为 300 KB，"
    "可以在 Chrome、Safari 和 Firefox 中运行。2026 年 11 月的测试覆盖了 4725 个公开文档，"
    "其中 99.1% 可以正常打开，平均每个文档耗时不到 1 毫秒。"
)
TEXT_D = (
    "他问：“这份文件是谁起草的？”我回答说：“是办公室的同事（小王）起草的，已经过三次修改。”"
    "他又问：“定稿了吗？”——没有人能给出确定的答复……于是会议决定：先由各部门提出意见，"
    "再统一修改；修改完成后，报主要负责人审定。"
)


def indent2(size=10.5):
    """First-line indent of two characters, as Word writes it for this size."""
    return {"firstLineChars": 200, "firstLine": round(size * 2 * 20)}


def body(text, **props):
    props.setdefault("ind", indent2())
    return p(text, **props)


def title(text, size=18, font="黑体"):
    return p(r(text, font=font, ascii="Arial", sz=size), jc="center", spacing={"before": 156, "after": 156})


# --------------------------------------------------------------------------- 01

def font_sizes():
    parts = [title("字号与字体对照")]
    parts.append(p("下表列出中文字号与磅值的对应关系。每一行使用它所说明的字号，文字为宋体，西文为 Times New Roman。"))
    for name, points in SIZES.items():
        parts.append(p(
            r(f"{name} {points:g}磅", font="黑体", ascii="Arial", sz=points),
            r(" 永和九年 Layout 2026", sz=points),
            jc="left",
        ))
    parts.append(p(r("常用字体", font="黑体", sz=14), spacing={"before": 312, "after": 156}))
    sample = "天地玄黄，宇宙洪荒。The quick brown fox jumps. 0123456789"
    for font, latin in (("宋体", "Times New Roman"), ("黑体", "Arial"), ("楷体", "Times New Roman"), ("仿宋", "Times New Roman")):
        parts.append(p(r(f"{font}：", font="黑体", sz=12), r(sample, font=font, ascii=latin, sz=12), jc="left"))
    parts.append(p(r("全部使用中文字体（西文也用宋体）：", font="黑体", sz=12), r(sample, font="宋体", ascii="宋体", sz=12), jc="left"))
    parts.append(p(r("同一行内混用字号", font="黑体", sz=14), spacing={"before": 312, "after": 156}))
    parts.append(p(
        "本段前半部分是五号字，", r("中间一句是二号字，", sz=22), "随后回到五号字。行高由该行最大的字决定，",
        r("小五号字", sz=9), "不会让行变矮。" + TEXT_A,
    ))
    write(OUT / "01-font-sizes.docx", "".join(parts))


# --------------------------------------------------------------------------- 02

def character_effects():
    h = lambda text: p(r(text, font="黑体", ascii="Arial", sz=14), spacing={"before": 234, "after": 78}, keep_next=True)
    line = lambda *runs: p(*runs, jc="left")
    parts = [title("字体效果")]
    parts += [
        h("粗体、斜体"),
        line("常规文字，", r("加粗文字", b=True), "，", r("倾斜文字", i=True), "，", r("加粗并倾斜", b=True, i=True),
             "，Regular ", r("Bold", b=True), " ", r("Italic", i=True), " ", r("Bold Italic", b=True, i=True), "。"),
        line(r("黑体加粗", font="黑体", b=True), "，", r("楷体加粗", font="楷体", b=True), "，", r("楷体倾斜", font="楷体", i=True),
             "，", r("仿宋加粗", font="仿宋", b=True), "。"),
        h("下划线"),
        line(r("单线", u="single"), " ", r("双线", u="double"), " ", r("粗线", u="thick"), " ", r("点线", u="dotted"), " ",
             r("虚线", u="dash"), " ", r("点划线", u="dotDash"), " ", r("波浪线", u="wave"), " ",
             r("红色下划线", u="single", u_color="FF0000"), " ", r("只给 字 加线", u="words"), "。"),
        line("合同中常见的填空：甲方（盖章）", r("                    ", u="single"), "　日期", r("        ", u="single"), "年",
             r("    ", u="single"), "月", r("    ", u="single"), "日"),
        h("删除线"),
        line(r("单删除线", strike=True), "，", r("双删除线", dstrike=True), "，", r("deleted text", strike=True), "。"),
        h("上标、下标"),
        line("面积 100 m", r("2", vert="superscript"), "，水的化学式 H", r("2", vert="subscript"), "O，注释",
             r("①", vert="superscript"), "，E = mc", r("2", vert="superscript"), "，第 1", r("st", vert="superscript"), " 名。"),
        h("着重号"),
        line("这句话里", r("最重要的几个字", em="dot"), "加了着重号，", r("comma 形式", em="comma"), "和",
             r("圆圈形式", em="circle"), "较少使用。"),
        h("颜色、突出显示、底纹、边框"),
        line(r("红色文字", color="FF0000"), "，", r("蓝色文字", color="0070C0"), "，", r("黄色突出显示", highlight="yellow"), "，",
             r("绿色突出显示", highlight="green"), "，", r("灰色底纹", shd="D9D9D9"), "，", r("字符边框", bdr=True), "。"),
        h("字符间距、缩放、位置"),
        line("标准间距，", r("加宽 2 磅的文字", spacing=40), "，", r("紧缩 1 磅的文字", spacing=-20), "，",
             r("缩放 150%", scale=150), "，", r("缩放 66%", scale=66), "，", r("提升 3 磅", position=6), "，",
             r("降低 3 磅", position=-6), "。"),
        h("大写与小型大写"),
        line("原文 OpencloudOffice Layout，", r("全部大写 OpencloudOffice Layout", caps=True), "，",
             r("小型大写 OpencloudOffice Layout", small_caps=True), "。"),
        h("效果叠加"),
        line(r("加粗、红色、双下划线、三号", b=True, color="C00000", u="double", sz=16), "，",
             r("楷体、倾斜、着重号", font="楷体", i=True, em="dot"), "，",
             r("黑体小四加粗黄色突出显示", font="黑体", b=True, sz=12, highlight="yellow"), "。"),
        h("隐藏文字"),
        line("这句话中间", r("（这几个字被设为隐藏，不应显示）", vanish=True), "有一段隐藏文字。"),
    ]
    write(OUT / "02-character-effects.docx", "".join(parts))


# --------------------------------------------------------------------------- 03

def indents_and_spacing():
    parts = [title("缩进、对齐与间距")]
    h = lambda text: p(r(text, font="黑体", ascii="Arial", sz=14), spacing={"before": 234, "after": 78}, keep_next=True)
    parts += [
        h("缩进"),
        p("【无缩进】" + TEXT_A),
        p("【首行缩进 2 字符】" + TEXT_A, ind=indent2()),
        p("【悬挂缩进 2 字符】" + TEXT_A, ind={"left": 420, "hangingChars": 200, "hanging": 420}),
        p("【左缩进 4 字符，右缩进 4 字符】" + TEXT_A, ind={"leftChars": 400, "left": 840, "rightChars": 400, "right": 840}),
        p("【左缩进 2 厘米，首行再缩进 2 字符】" + TEXT_A, ind={"left": 1134, "firstLineChars": 200, "firstLine": 420}),
        h("对齐"),
        p("【左对齐】" + TEXT_C, jc="left"),
        p("【两端对齐】" + TEXT_C, jc="both"),
        p("【居中】" + TEXT_B, jc="center"),
        p("【右对齐】" + TEXT_B, jc="right"),
        p("【分散对齐】" + TEXT_B, jc="distribute"),
        h("行距"),
        p("【单倍行距】" + TEXT_A, ind=indent2()),
        p("【1.5 倍行距】" + TEXT_A, ind=indent2(), spacing={"line": 360, "lineRule": "auto"}),
        p("【2 倍行距】" + TEXT_A, ind=indent2(), spacing={"line": 480, "lineRule": "auto"}),
        p("【固定值 20 磅】" + TEXT_A, ind=indent2(), spacing={"line": 400, "lineRule": "exact"}),
        p("【最小值 12 磅】" + TEXT_A, ind=indent2(), spacing={"line": 240, "lineRule": "atLeast"}),
        p("【多倍行距 1.25，不对齐网格】" + TEXT_A, ind=indent2(), spacing={"line": 300, "lineRule": "auto"}, snap=False),
        h("段落间距"),
        p("【段前 0.5 行，段后 0.5 行】" + TEXT_B, ind=indent2(), spacing={"beforeLines": 50, "before": 156, "afterLines": 50, "after": 156}),
        p("【段前 12 磅，段后 6 磅】" + TEXT_B, ind=indent2(), spacing={"before": 240, "after": 120}),
        p("【段前段后均为 0】" + TEXT_B, ind=indent2()),
        h("制表位"),
        p("姓名\t部门\t工号", tabs=[("left", 2100, None), ("left", 4200, None)], jc="left"),
        p("张三\t技术部\t10086", tabs=[("left", 2100, None), ("left", 4200, None)], jc="left"),
        p("第一章 总则\t1", tabs=[("right", 8306, "dot")], jc="left"),
        p("第二章 文档格式\t12", tabs=[("right", 8306, "dot")], jc="left"),
        p("甲方（盖章）：\t乙方（盖章）：", tabs=[("left", 4153, None)], jc="left"),
    ]
    write(OUT / "03-indents-and-spacing.docx", "".join(parts))


# --------------------------------------------------------------------------- 04

def report():
    """A report with numbered headings (1, 1.1, 1.1.1) and three kinds of list."""
    numbering_xml = numbering(
        [
            abstract_num(0, [
                level(0, "decimal", "%1", left=432, hanging=432, pstyle="1"),
                level(1, "decimal", "%1.%2", left=576, hanging=576, pstyle="2"),
                level(2, "decimal", "%1.%2.%3", left=720, hanging=720, pstyle="3"),
            ]),
            abstract_num(1, [
                level(0, "bullet", "●", left=840, hanging=420, rpr_xml=rpr(font="宋体", ascii="宋体")),
                level(1, "bullet", "○", left=1260, hanging=420, rpr_xml=rpr(font="宋体", ascii="宋体")),
            ], multi="hybridMultilevel"),
            abstract_num(2, [level(0, "decimal", "（%1）", left=840, hanging=420)], multi="singleLevel"),
            abstract_num(3, [level(0, "decimalEnclosedCircle", "%1", left=840, hanging=420)], multi="singleLevel"),
        ],
        [num(1, 0), num(2, 1), num(3, 2), num(4, 3), num(5, 2), num(6, 2, start_override(0, 1))],
    )
    h1 = lambda text: p(text, style="1")
    h2 = lambda text: p(text, style="2")
    h3 = lambda text: p(text, style="3")
    item = lambda text, num_id, ilvl=0: p(text, num=(num_id, ilvl))
    parts = [
        title("文档渲染项目阶段报告", size=22),
        h1("项目概述"),
        body(TEXT_B),
        h2("背景"),
        body(TEXT_A),
        h2("目标"),
        body("本阶段的目标分为三项："),
        item("读取：准确解析文档结构和样式继承。", 3),
        item("排版：计算每一行、每一页的位置，结果与原软件一致。", 3),
        item("显示：在浏览器和移动端使用同一套内核输出页面。", 3),
        h1("技术方案"),
        h2("读取内核"),
        h3("样式继承"),
        body("一段文字的最终格式来自文档默认值、段落样式、字符样式和直接格式四层设置，逐层叠加后才能得到。"),
        h3("编号"),
        body("编号的文字不保存在段落里，而是由编号定义和段落在列表中的位置计算出来。标题的编号通过样式与编号定义关联。"),
        h2("排版引擎"),
        body("排版引擎需要处理以下内容："),
        item("字体度量与替换", 2),
        item("断行", 2),
        item("避头尾：句号、逗号等不能出现在行首", 2, 1),
        item("标点挤压：相邻的全角标点各占半个字宽", 2, 1),
        item("行高与文档网格", 2),
        item("分页", 2),
        h1("测试结果"),
        h2("公开语料"),
        body(TEXT_C),
        h2("中文文档"),
        body("中文文档的测试重点如下："),
        item("标题层级与多级编号", 4),
        item("首行缩进与悬挂缩进", 4),
        item("字号、字体与字体效果", 4),
        item("文档网格下的行高", 4),
        body("下面两项使用同一个编号定义的另一个实例。同一定义的各个实例共用计数，所以它们接着前文的（3）继续编号："),
        item("标题编号与正文编号互不影响。", 5),
        item("同一编号定义的不同实例接续计数。", 5),
        body("下面两项的实例带有“重新开始编号”的标记，因此从（1）开始："),
        item("重新开始后的第一项。", 6),
        item("重新开始后的第二项。", 6),
        h1("下一步计划"),
        body(TEXT_A),
    ]
    write(OUT / "04-report-headings.docx", "".join(parts), styles=default_styles(heading_num=1), numbering_xml=numbering_xml)


# --------------------------------------------------------------------------- 05

def official_notice():
    """An official notice in the layout of GB/T 9704-2012: 3 号仿宋 body, fixed
    line height, level headings in 黑体 / 楷体, numbers typed as text."""
    line = {"line": 579, "lineRule": "exact"}
    fs = dict(font="仿宋", ascii="仿宋", sz=16)
    ind = {"firstLineChars": 200, "firstLine": 640}
    text = lambda t, **kw: p(r(t, **fs), ind=ind, spacing=line, **kw)
    parts = [
        p(r("某某市数据管理局文件", font="方正小标宋简体", ascii="方正小标宋简体", sz=36, color="FF0000", scale=80),
          jc="center", spacing={"before": 600, "after": 300, "line": 1000, "lineRule": "exact"}),
        p(r("某数发〔2026〕12 号", **fs), jc="center", spacing=line, border_bottom=(24, "FF0000", 6)),
        p(spacing=line),
        p(r("关于进一步规范办公文档格式的通知", font="方正小标宋简体", ascii="方正小标宋简体", sz=22), jc="center",
          spacing={"line": 660, "lineRule": "exact"}),
        p(spacing=line),
        p(r("各区、县人民政府，市政府各部门，各直属单位：", **fs), spacing=line),
        text("为提高公文处理的质量和效率，根据《党政机关公文格式》（GB/T 9704—2012）的有关规定，现就进一步规范办公文档格式的有关事项通知如下。"),
        p(r("一、总体要求", font="黑体", ascii="黑体", sz=16), ind=ind, spacing=line),
        text("各单位制发的文件应当使用统一的版式。正文使用 3 号仿宋体字，每面排 22 行，每行排 28 个字，并撑满版心。特定情况可以作适当调整。"),
        p(r("二、主要内容", font="黑体", ascii="黑体", sz=16), ind=ind, spacing=line),
        p(r("（一）标题层次。", font="楷体", ascii="楷体", sz=16),
          r("文中结构层次序数依次可以用“一、”“（一）”“1.”“（1）”标注；一般第一层用黑体字、第二层用楷体字、第三层和第四层用仿宋体字标注。", **fs),
          ind=ind, spacing=line),
        p(r("（二）页面设置。", font="楷体", ascii="楷体", sz=16),
          r("公文用纸采用 A4 型纸，天头（上白边）为 37 mm，订口（左白边）为 28 mm，版心尺寸为 156 mm × 225 mm。", **fs),
          ind=ind, spacing=line),
        text("1. 页码一般用 4 号半角宋体阿拉伯数字，编排在公文版心下边缘之下。"),
        text("2. 附件应当另面编排，并在版记之前，与公文正文一起装订。"),
        text("（1）附件标题居中编排在版心第三行。"),
        text("（2）附件顺序号和附件标题应当与附件说明的表述一致。"),
        p(r("三、工作要求", font="黑体", ascii="黑体", sz=16), ind=ind, spacing=line),
        text("各单位要高度重视，明确责任人，于 2026 年 12 月 31 日前完成现有模板的修订。执行中遇到的问题，请及时向市数据管理局办公室反映。"),
        p(spacing=line),
        p(r("附件：1. 办公文档格式模板", **fs), ind=ind, spacing=line),
        p(r("2. 常见格式问题及处理办法", **fs), ind={"left": 1600}, spacing=line),
        p(spacing=line),
        p(spacing=line),
        p(r("某某市数据管理局", **fs), jc="right", ind={"rightChars": 400, "right": 1280}, spacing=line),
        p(r("2026 年 11 月 12 日", **fs), jc="right", ind={"rightChars": 400, "right": 1280}, spacing=line),
        p(r("（此件公开发布）", **fs), ind=ind, spacing=line),
    ]
    sect = section(top=2098, bottom=1984, left=1588, right=1474, grid=None)
    write(OUT / "05-official-notice.docx", "".join(parts), sect=sect, styles=default_styles(body_font="仿宋", body_size=16))


# --------------------------------------------------------------------------- 06

def contract():
    """A contract: articles numbered 第一条, 第二条 ... inline with the text."""
    numbering_xml = numbering(
        [abstract_num(0, [
            level(0, "chineseCounting", "第%1条", left=0, first_line=480, suff="space", rpr_xml=rpr(font="黑体", ascii="黑体")),
            level(1, "chineseCounting", "（%2）", left=0, first_line=480, suff="nothing"),
            level(2, "decimal", "%3.", left=0, first_line=480, suff="space"),
        ])],
        [num(1, 0)],
    )
    spacing = {"line": 360, "lineRule": "auto"}
    art = lambda *runs: p(*runs, num=(1, 0), spacing=spacing)
    sub = lambda text: p(text, num=(1, 1), spacing=spacing)
    subsub = lambda text: p(text, num=(1, 2), spacing=spacing)
    plain = lambda *runs, **kw: p(*runs, spacing=spacing, **kw)
    blank = lambda n: r(" " * n, u="single")
    parts = [
        p(r("技术服务合同", font="黑体", ascii="黑体", sz=22), jc="center", spacing={"before": 312, "after": 312}),
        plain("合同编号：", blank(20), jc="right"),
        plain("甲方（委托方）：", blank(36), jc="left"),
        plain("乙方（服务方）：", blank(36), jc="left"),
        plain("甲乙双方经平等协商，就甲方委托乙方提供文档处理技术服务事宜，根据《中华人民共和国民法典》的有关规定，订立本合同，共同遵守。",
              ind={"firstLineChars": 200, "firstLine": 480}),
        art(r("服务内容", b=True)),
        sub("乙方为甲方提供在线文档预览与编辑软件开发工具包（以下简称“SDK”）及配套技术支持。"),
        sub("服务范围包括："),
        subsub("Word、Excel、PDF 三类文档的在线显示；"),
        subsub("文档的修订、批注与内容增删；"),
        subsub("与甲方现有对象存储系统的对接。"),
        art(r("服务期限", b=True)),
        plain("本合同服务期限自", blank(6), "年", blank(4), "月", blank(4), "日起至", blank(6), "年", blank(4), "月", blank(4),
              "日止，共计 12 个月。", ind={"firstLineChars": 200, "firstLine": 480}),
        art(r("费用及支付方式", b=True)),
        p("本合同服务费用总额为人民币（大写）", blank(24), "元整（¥", blank(12), "）。", num=(1, 1), spacing=spacing),
        sub("甲方应于本合同生效之日起 10 个工作日内支付合同总额的 50%，验收合格后 10 个工作日内支付剩余款项。"),
        art(r("双方的权利和义务", b=True)),
        sub("甲方应当按约定提供必要的测试文档和运行环境，并对其提供资料的合法性负责。"),
        sub("乙方应当保证所提供的 SDK 不侵犯任何第三方的知识产权；对在履行合同过程中知悉的甲方文档内容负有保密义务。"),
        art(r("违约责任", b=True)),
        plain("任何一方违反本合同约定，给对方造成损失的，应当承担赔偿责任。因不可抗力不能履行合同的，根据不可抗力的影响，部分或者全部免除责任。",
              ind={"firstLineChars": 200, "firstLine": 480}),
        art(r("争议解决", b=True)),
        plain("因本合同引起的或与本合同有关的任何争议，双方应友好协商解决；协商不成的，任何一方均可向甲方所在地人民法院提起诉讼。",
              ind={"firstLineChars": 200, "firstLine": 480}),
        art(r("其他", b=True)),
        plain("本合同一式两份，甲乙双方各执一份，自双方签字盖章之日起生效。", ind={"firstLineChars": 200, "firstLine": 480}),
        plain(),
        plain("甲方（盖章）：\t乙方（盖章）：", tabs=[("left", 4320, None)], jc="left"),
        plain("法定代表人：\t法定代表人：", tabs=[("left", 4320, None)], jc="left"),
        plain("日期：    年  月  日\t日期：    年  月  日", tabs=[("left", 4320, None)], jc="left"),
    ]
    write(OUT / "06-contract.docx", "".join(parts), numbering_xml=numbering_xml, styles=default_styles(body_size=12))


# --------------------------------------------------------------------------- 07

def regulation():
    """Rules and regulations: chapters (第一章) with articles (第一条) that keep
    counting across chapters instead of restarting in each one."""
    chapter_style = style("zhang", "章标题", based_on="a", next_style="a",
                          ppr_xml=ppr(keep_next=True, num=(1, 0), spacing={"before": 312, "after": 312}, jc="center", outline=0),
                          rpr_xml=rpr(font="黑体", ascii="黑体", sz=16))
    numbering_xml = numbering(
        [abstract_num(0, [
            level(0, "chineseCounting", "第%1章", left=0, hanging=0, suff="space", pstyle="zhang", jc="left"),
            level(1, "chineseCounting", "第%2条", left=0, first_line=480, suff="space", restart=0, rpr_xml=rpr(b=True)),
            level(2, "chineseCounting", "（%3）", left=0, first_line=480, suff="nothing"),
        ])],
        [num(1, 0)],
    )
    spacing = {"line": 400, "lineRule": "exact"}
    chapter = lambda text: p(text, style="zhang")
    article = lambda text: p(text, num=(1, 1), spacing=spacing)
    clause = lambda text: p(text, num=(1, 2), spacing=spacing)
    parts = [
        p(r("公司文档管理办法", font="方正小标宋简体", ascii="方正小标宋简体", sz=22), jc="center", spacing={"before": 312, "after": 156}),
        p(r("（2026 年 11 月修订）", font="楷体", ascii="楷体", sz=12), jc="center", spacing={"after": 312}),
        chapter("总则"),
        article("为规范公司文档的起草、审核、存储和使用，提高工作效率，保障信息安全，根据国家有关法律法规，结合公司实际，制定本办法。"),
        article("本办法适用于公司各部门及全体员工在工作中形成的各类电子文档，包括但不限于制度文件、合同、报告、会议纪要和技术资料。"),
        article("文档管理遵循以下原则："),
        clause("统一规范，分级负责；"),
        clause("安全保密，便于利用；"),
        clause("全程留痕，可以追溯。"),
        chapter("文档格式"),
        article("正式文件应当使用公司统一模板。正文使用小四号宋体，行距为固定值 20 磅，首行缩进 2 个字符；一级标题使用三号黑体，居中排列。"),
        article("文档中的结构层次序数，第一层为“第一章”，第二层为“第一条”，第三层为“（一）”。条的序号在全文中连续编排，不因分章而重新起算。"),
        article("涉及数量、金额、日期的内容，应当使用阿拉伯数字；涉及法律条文引用的，应当写明全称。"),
        chapter("审核与发布"),
        article("文档起草完成后，起草人应当使用修订模式提交审核。审核人的修改意见以修订和批注的形式保留，直至定稿。"),
        article("文档经部门负责人审核、分管领导批准后方可发布。发布版本应当接受全部修订并删除批注。"),
        chapter("附则"),
        article("本办法由公司办公室负责解释。"),
        article("本办法自发布之日起施行。"),
    ]
    write(OUT / "07-regulation.docx", "".join(parts), numbering_xml=numbering_xml,
          styles=default_styles(body_size=12, extra=chapter_style))


# --------------------------------------------------------------------------- 08

def thesis():
    """A thesis chapter: 小四 body at 1.5 line spacing, numbered headings with a
    space after the number, an English abstract, and a reference list."""
    numbering_xml = numbering(
        [
            abstract_num(0, [
                level(0, "decimal", "%1", left=0, hanging=0, suff="space", pstyle="1"),
                level(1, "decimal", "%1.%2", left=0, hanging=0, suff="space", pstyle="2"),
                level(2, "decimal", "%1.%2.%3", left=0, hanging=0, suff="space", pstyle="3"),
            ]),
            abstract_num(1, [level(0, "decimal", "[%1]", left=420, hanging=420)], multi="singleLevel"),
        ],
        [num(1, 0), num(2, 1)],
    )
    styles = default_styles(body_size=12, heading_num=1)
    # Thesis templates restyle the headings: 黑体, not bold, smaller, left aligned.
    styles = styles.replace('<w:b/><w:bCs/><w:kern w:val="44"/><w:sz w:val="44"/><w:szCs w:val="44"/>',
                            '<w:rFonts w:ascii="Arial" w:hAnsi="Arial" w:eastAsia="黑体"/><w:b w:val="0"/><w:sz w:val="32"/><w:szCs w:val="32"/>')
    spacing = {"line": 360, "lineRule": "auto"}
    para = lambda *runs: p(*runs, ind={"firstLineChars": 200, "firstLine": 480}, spacing=spacing)
    parts = [
        p(r("摘  要", font="黑体", ascii="黑体", sz=15), jc="center", spacing={"before": 240, "after": 240}),
        para("办公文档的在线显示是协同办公系统的基础能力。现有方案或者依赖服务端转换，或者借用浏览器自身的排版能力，"
             "前者成本高、延迟大，后者无法保证分页与原文档一致。本文研究一种完全在客户端运行的文档排版方法，"
             "重点解决中文文档中文档网格、标点处理和字体替换带来的版式偏差问题。"),
        p(r("关键词：", font="黑体", ascii="黑体", sz=12), r("文档排版；文档网格；标点挤压；字体替换；WebAssembly", sz=12), spacing=spacing),
        p(spacing=spacing),
        p(r("Abstract", ascii="Arial", b=True, sz=15), jc="center", spacing={"before": 240, "after": 240}),
        para("Rendering office documents in the browser is a basic capability of collaborative office systems. "
             "Existing solutions either rely on server-side conversion, which is costly and slow, or delegate layout to the "
             "browser, which cannot reproduce the pagination of the original document. This thesis studies a layout method "
             "that runs entirely on the client."),
        p(r("Key words: ", b=True, sz=12), r("document layout; document grid; punctuation compression; font substitution", sz=12), spacing=spacing),
        p(page_break()),
        p("绪论", style="1"),
        p("研究背景", style="2"),
        para(TEXT_B),
        para("以一份 20 页的技术报告为例：如果每页多排了一行，全文就会少一页；如果某一行因为字体宽度不同而提前换行，"
             "后面所有段落的位置都会随之改变。因此排版的正确性必须逐行验证，而不能只看整体观感。"),
        p("国内外研究现状", style="2"),
        p("服务端转换", style="3"),
        para("这类方案以 LibreOffice 为代表", r("[1]", vert="superscript"), "，在服务器上把文档转换为 PDF 或图片后再发送给浏览器。"
             "其优点是实现简单，缺点是版式受转换程序限制，且每次预览都要占用服务器资源。"),
        p("客户端渲染", style="3"),
        para("这类方案在浏览器内解析并显示文档", r("[2-3]", vert="superscript"), "。早期实现直接把文档转换为 HTML，"
             "由浏览器完成排版，无法精确分页；近年来的实现改为自行计算每个字的位置，再绘制到画布上。"),
        p("本文的主要工作", style="2"),
        para(TEXT_A),
        p("参考文献", style="1"),
        p("The Document Foundation. LibreOffice Writer Guide[M]. 2024.", num=(2, 0), jc="left", spacing=spacing),
        p("Ecma International. ECMA-376: Office Open XML File Formats[S]. 5th ed. Geneva: Ecma International, 2021.", num=(2, 0), jc="left", spacing=spacing),
        p("万维网联盟. 中文排版需求[EB/OL]. (2023-08-01)[2026-11-12]. https://www.w3.org/TR/clreq/.", num=(2, 0), jc="left", spacing=spacing),
        p("国家质量监督检验检疫总局. 党政机关公文格式: GB/T 9704—2012[S]. 北京: 中国标准出版社, 2012.", num=(2, 0), jc="left", spacing=spacing),
    ]
    write(OUT / "08-thesis.docx", "".join(parts), numbering_xml=numbering_xml, styles=styles)


# --------------------------------------------------------------------------- 09

def number_formats():
    """One list per number format, with starts chosen to show tens and hundreds."""
    formats = [
        ("decimal", "%1.", "阿拉伯数字"),
        ("decimalZero", "%1.", "前导零"),
        ("upperRoman", "%1.", "大写罗马数字"),
        ("lowerRoman", "%1.", "小写罗马数字"),
        ("upperLetter", "%1.", "大写字母"),
        ("lowerLetter", "%1)", "小写字母"),
        ("chineseCounting", "%1、", "中文数字"),
        ("chineseCountingThousand", "%1、", "中文数字（千）"),
        ("chineseLegalSimplified", "%1、", "中文大写数字"),
        ("ideographTraditional", "%1、", "天干"),
        ("ideographZodiac", "%1、", "地支"),
        ("decimalEnclosedCircle", "%1", "带圈数字"),
        ("decimalFullWidth", "%1．", "全角数字"),
        ("decimalEnclosedParen", "%1", "带括号数字"),
        ("ordinal", "%1", "英文序数"),
        ("cardinalText", "%1.", "英文基数词"),
    ]
    abstracts, nums, parts = [], [], [title("编号格式")]
    parts.append(p("每种格式列出三组：从 1 开始、从 9 开始、从 99 开始，用来观察十位和百位的写法。"))
    num_id = 0
    for index, (fmt, text, label) in enumerate(formats):
        abstracts.append(abstract_num(index, [level(0, fmt, text, left=1260, hanging=1260)], multi="singleLevel"))
        parts.append(p(r(f"{label}（{fmt}）", font="黑体", ascii="Arial", sz=12), spacing={"before": 156}, keep_next=True))
        for start in (1, 9, 99):
            num_id += 1
            nums.append(num(num_id, index, start_override(0, start)))
            for offset in range(3):
                parts.append(p(f"第 {start + offset} 项", num=(num_id, 0), jc="left"))
    base = len(formats)
    abstracts.append(abstract_num(base, [
        level(0, "chineseCounting", "%1、", left=420, hanging=420),
        level(1, "chineseCounting", "（%2）", left=840, hanging=630),
        level(2, "decimal", "%3.", left=1260, hanging=420),
        level(3, "decimal", "（%4）", left=1680, hanging=630),
        level(4, "decimalEnclosedCircle", "%5", left=2100, hanging=420),
    ]))
    abstracts.append(abstract_num(base + 1, [
        level(0, "chineseCounting", "第%1章", left=840, hanging=840),
        level(1, "decimal", "%1.%2", left=840, hanging=840, legal=True),
        level(2, "decimal", "%1.%2.%3", left=840, hanging=840),
    ]))
    abstracts.append(abstract_num(base + 2, [
        level(0, "decimal", "%1.", left=840, hanging=420, jc="right"),
    ], multi="singleLevel"))
    multi, legal, right = num_id + 1, num_id + 2, num_id + 3
    nums += [num(multi, base), num(legal, base + 1), num(right, base + 2, start_override(0, 8))]
    parts.append(p(r("公文常用的多级编号", font="黑体", ascii="Arial", sz=12), spacing={"before": 156}, keep_next=True))
    for ilvl, text in ((0, "第一层"), (1, "第二层"), (2, "第三层"), (3, "第四层"), (4, "第五层"), (4, "第五层第二项"),
                       (2, "回到第三层"), (1, "回到第二层，下一层重新计数"), (2, "第三层从 1 开始"), (0, "第一层第二项"), (1, "第二层重新计数")):
        parts.append(p(text, num=(multi, ilvl), jc="left"))
    parts.append(p(r("章用中文数字，节要求沿用的章号改用阿拉伯数字（isLgl）", font="黑体", ascii="Arial", sz=12), spacing={"before": 156}, keep_next=True))
    for ilvl, text in ((0, "章的编号是中文数字"), (1, "节设置了 isLgl，章号显示为阿拉伯数字"), (2, "小节没有设置，章号仍是中文数字"),
                       (1, "下一节"), (0, "下一章"), (1, "节重新计数")):
        parts.append(p(text, num=(legal, ilvl), jc="left"))
    parts.append(p(r("编号右对齐（个位对齐）", font="黑体", ascii="Arial", sz=12), spacing={"before": 156}, keep_next=True))
    for n in range(8, 12):
        parts.append(p(f"第 {n} 项，编号的右边缘对齐，因此个位数与两位数的句点在同一位置。", num=(right, 0), jc="left"))
    write(OUT / "09-number-formats.docx", "".join(parts), numbering_xml=numbering(abstracts, nums))


# --------------------------------------------------------------------------- 10

def punctuation():
    parts = [title("标点与断行")]
    h = lambda text: p(r(text, font="黑体", ascii="Arial", sz=14), spacing={"before": 234, "after": 78}, keep_next=True)
    parts += [
        h("引号、括号与连续标点"),
        body(TEXT_D),
        body("《党政机关公文格式》（GB/T 9704—2012）规定：“公文用纸采用 GB/T 148 中规定的 A4 型纸”；“一般每面排 22 行，每行排 28 个字”。"
             "（引文末尾的句号、引号和括号连在一起时，各占半个字的宽度。）"),
        h("行首行尾禁则"),
        body("下面这一行经过设计，使逗号恰好落在行尾之后的位置：一二三四五六七八九十一二三四五六七八九十一二三四五，六七八九十。"
             "句号、逗号、顿号、分号、冒号、后引号和后括号不能出现在行首；前引号、前括号不能出现在行尾。"),
        body("同样的文字左对齐时：一二三四五六七八九十一二三四五六七八九十一二三四五六，七八九十。行尾会留出不齐的空白，两端对齐时则由字间距吸收。", jc="left"),
        h("中西文混排"),
        body(TEXT_C),
        body("较长的西文单词不能在中间断开，例如 internationalization 和 https://www.example.com/documents/2026/layout-report.html "
             "这样的网址；当一行剩余的位置放不下整个单词时，单词整体移到下一行，上一行的字间距被拉大。"),
        body("数字与单位：长度 21.0 cm × 29.7 cm，重量 80 g/m²，金额 ¥1,234,567.89，百分比 99.1%，日期 2026-11-12，时间 14:30:00，"
             "电话 010-12345678，温度 −5 ℃ 至 35 ℃。"),
        h("破折号与省略号"),
        body("破折号——占两个字的位置——不能从中间断开；省略号……也是如此。连接号用于表示范围，如 2020—2026 年、第 3～5 页、北京—上海。"),
        h("全角与半角"),
        body("全角标点：，。；：？！（）“”‘’《》【】、——……"),
        body("半角标点：,.;:?!()\"\"''<>[]/--..."),
        body("全角字母数字：ＡＢＣ１２３；半角字母数字：ABC123；半角片假名与特殊符号：①②③ ㈠㈡㈢ Ⅰ Ⅱ Ⅲ ℃ № ※ → ←。"),
    ]
    write(OUT / "10-punctuation.docx", "".join(parts))


# --------------------------------------------------------------------------- 11

def handbook():
    """A handbook with headers, footers and page numbers, as long documents
    have them: a cover without either, front matter numbered in roman, and a
    body that starts again from page 1 with a header of its own.

    Odd and even pages differ. The body's own header, on its odd pages, is
    three lines and reaches below the top margin, so the text of those pages
    starts lower than on the even pages, whose header is one line. The body
    names no even header and no footers: it has those of the front matter.
    """
    small = lambda text, **fmt: r(text, sz=9, **fmt)
    rule = (6, "auto", 1)
    head = lambda *runs, jc: p(*runs, jc=jc, border_bottom=rule, snap=False)
    foot = lambda *runs, jc: p(*runs, jc=jc, snap=False)
    stories = {
        # Front matter, odd pages.
        "header1": head(small("前言"), jc="right"),
        # Even pages, throughout.
        "header2": head(small("办公文档排版手册"), jc="left"),
        # The body, odd pages: taller than the top margin leaves room for.
        "header3": p(small("办公文档排版手册"), jc="right", snap=False)
        + p(small("第二版"), jc="right", snap=False)
        + head(small("正文"), jc="right"),
        "footer1": foot(small("第 "), field("PAGE", "1", sz=9), small(" 页"), jc="right"),
        "footer2": foot(small("第 "), field("PAGE", "2", sz=9), small(" 页 共 "), field("NUMPAGES", "9", sz=9), small(" 页"), jc="left"),
    }
    front = section(headers={"default": "header1", "even": "header2"}, footers={"default": "footer1", "even": "footer2"},
                    title_page=True, page_numbers=("lowerRoman", None))
    body_section = section(headers={"default": "header3"}, page_numbers=("decimal", 1))

    paragraph = lambda text: body(text)
    texts = (TEXT_A, TEXT_B, TEXT_C, TEXT_D)
    parts = [
        p(spacing={"before": 2400}),
        p(r("办公文档排版手册", font="黑体", ascii="Arial", sz=26), jc="center", spacing={"after": 312}),
        p(r("第二版", font="楷体", sz=16), jc="center"),
        p(spacing={"before": 4800}),
        p(r("文档工程组", sz=14), jc="center"),
        p(r("2026 年 11 月", sz=14), jc="center"),
        p(page_break()),
        title("前  言"),
    ]
    for number in range(1, 17):
        parts.append(paragraph(f"（前言第 {number} 段）" + texts[number % 4]))
    # The last paragraph of the front matter carries its section properties.
    parts.append(f"<w:p><w:pPr>{front}</w:pPr></w:p>")
    for chapter, name in enumerate(("版心与页边距", "页眉、页脚与页码", "分节与页码格式"), start=1):
        parts.append(title(f"第{'一二三'[chapter - 1]}章  {name}", size=16))
        for number in range(1, 15):
            parts.append(paragraph(f"（{chapter}.{number}）" + texts[(chapter + number) % 4]))
    write(OUT / "11-header-footer.docx", "".join(parts), sect=body_section, stories=stories, even_and_odd=True)


def main():
    for build in (font_sizes, character_effects, indents_and_spacing, report, official_notice, contract, regulation,
                  thesis, number_formats, punctuation, handbook):
        build()
    for path in sorted(OUT.glob("*.docx")):
        print(f"{path.stat().st_size:7d}  {path.relative_to(OUT.parent.parent)}")


if __name__ == "__main__":
    main()
