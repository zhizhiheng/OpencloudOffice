# Word 实测行为

规范（ECMA-376）规定了合法文档长什么样，没有规定应用程序遇到不合法或有歧义的内容时怎么办。Word 在这些地方有自己稳定的做法，而真实世界的文档（尤其是第三方工具生成的）经常落在这些地方。要显示得和 Word 一样，就得照 Word 的做法读。

本文记录内核已经实现的、与"只看规范会怎么写"不同的规则，以及每条规则的依据有多可靠。

## 依据和验证方式

我们没有直接运行 Word。规则来自 docx4j 项目公开的材料（测量于 2026 年 9 月至 10 月，Word 365），验证方式分三种，下表逐条标明：

| 标记 | 含义 |
|---|---|
| 配对 | 有探测文档 `X.docx` 和 Word 另存的 `X-word365.docx`。Word 另存时只写规范的标记，所以后者记录了 Word 的理解。`tools/corpus/word_pairs.py` 检查内核读两者的结果是否一致，该规则已通过 |
| PDF | 我们读取了 Word 导出的 PDF（docx4j 的 `docx4j-layout-fidelity/goldens/word/`），从中直接看到结果 |
| 记录 | 只见于 docx4j 的测量记录（`docs/developer/change-requests/`），我们没有独立验证 |

每条规则各有一个手写的回归测试，在 `crates/oco-docx/tests/reader.rs` 中。

## 属性值

| 规则 | 验证 | 测试 |
|---|---|---|
| 开关属性（粗体、斜体等）只有值恰好是 `false`、`off`、`0` 时为关，区分大小写、不去空格。其他一切都是开，包括空串、`FALSE`、` false`、`no` | 配对 | `on_off_is_off_only_for_three_exact_spellings` |
| 段落样式和字符样式都开启同一个开关属性时，两者抵消，结果是关。样式里明确写"关"不参与抵消 | 配对、PDF | `toggle_properties_flip_between_paragraph_and_character_styles` |
| 文档默认值里的开关属性只在没有任何样式提及它时生效。默认值为粗体、段落样式写了"不加粗"时，结果是不加粗 | PDF | `document_default_toggles_apply_only_when_no_style_speaks` |
| 颜色接受六位十六进制，大小写均可，可带前导 `#`，两端空格会去掉 | 配对 | `invalid_property_values_leave_the_style_value_in_place` |
| 自动颜色必须恰好写作 `auto`。`Auto`、` auto` 无效 | 配对 | 同上 |
| 无效的颜色值等同于没写，样式里的颜色会显示出来。`F00`、`0xFF0000`、空串都属此类 | 配对 | 同上 |
| 颜色可以用经典 HTML 颜色名，不区分大小写。灰色只认 `gray`、`darkgray`、`lightgrey` 这一套拼法 | 配对（约 60 个名称实测，其余按同一张表推定） | `reads_colour_names` |
| 下划线的值不在枚举内（包括大小写不符、空串、不带值）等同于没写，样式里的下划线会显示出来 | 配对 | `invalid_property_values_leave_the_style_value_in_place` |
| 下划线的值是六个开关词之一（`true`、`on`、`1`、`false`、`off`、`0`）时，一律读作"无下划线"，包括 `true` | 配对 | 同上 |
| 高亮和上下标的值不在枚举内等同于没写 | 配对 | 同上 |
| 字符样式里的高亮不生效；段落样式里的高亮生效 | 配对 | `a_character_style_contributes_no_highlight` |
| 行距：只写了行距值而没写规则时，规则是"倍数"，不继承下层样式的"固定值"；只写段后间距时，行距和规则都保持不变 | 记录 | `a_line_height_without_a_rule_resets_the_rule` |

关于第三条：按我们对规范 17.7.3 节的理解，默认值开启的属性应当始终开启。Word 的 PDF 显示并非如此。规范原文没有在本次工作中重新核对，但无论原文如何，内核以 Word 的表现为准。

## 数值

| 规则 | 验证 | 测试 |
|---|---|---|
| 长度可以带单位（`36pt`、`0.5in`、`2.54cm`）。Word 保存 ISO Strict 格式时就这样写 | 语料中的 Word 文档 | `reads_lengths_written_with_units` |
| 整数可带正负号和小数部分，小数部分直接截断而不是四舍五入（`120.5` 读作 120）。没有数字时（空串、`-`、`.5`）读作 0 | 记录 | `reads_integers_as_word_does` |

带单位的长度是规范内的合法写法。列在这里是因为只按整数解析是常见的疏漏，后果是整个文档的缩进和间距全部丢失，本内核的第一版就有这个问题。

## 结构

| 规则 | 验证 | 测试 |
|---|---|---|
| 兼容性标记（`mc:AlternateContent`）取第一个所需命名空间全部认识的分支；都不认识时取后备分支；没有后备分支则什么都不读 | 配对 | `takes_the_first_compatibility_choice_word_understands` |
| 未知的 `w:` 命名空间元素不会被跳过，其子元素被当作直接出现在该位置，可以嵌套。其他命名空间的未知元素连同内容一起忽略 | 配对 | `reads_through_unknown_elements_of_the_main_namespace` |
| 内容控件和自定义 XML 元素可以包裹整行或整个单元格 | 配对；语料中有真实文档 | `content_controls_can_wrap_rows_and_cells` |
| 本应只出现一次的元素重复出现时全部读取：多个正文依次连接，多个属性元素按属性合并，多个控件内容依次连接 | 配对 | `merges_what_should_appear_once_but_appears_twice` |
| 换行、制表符等本应在文字块内的元素，直接出现在段落或超链接里时同样生效 | 配对 | `reads_run_content_placed_outside_a_run` |
| 文字元素内部嵌套的换行和文字元素会被读取，其他嵌套元素被丢弃 | 配对 | `reads_breaks_and_text_nested_inside_text` |
| 主文档部件的根可以是兼容性标记、正文、段落或未知元素，Word 都能读出内容；分支里有多个文档时依次连接 | 配对 | `reads_main_parts_with_an_unusual_root` |
| 包内有多个主文档关系时取最后一个 | 配对 | `the_last_main_document_relationship_wins` |
| 由样式带来的编号：级别取样式自己写的（没写为第一级），不取"指向该样式"的那一级；每一级只属于一个样式，其余样式的段落没有编号也不计数，但仍取该级的缩进 | PDF | `a_list_level_numbers_the_paragraphs_of_one_style_only` |
| 文档没有主题部件时，西文的主题字体引用取 Office 主题的字体：正文 Calibri，标题 Cambria | PDF | `theme_fonts_fall_back_to_the_office_theme_when_there_is_no_theme_part` |
| 表格样式的段落和字符格式排在文档默认值之后、段落样式之前。名为 Normal Table 的样式是内置的：左右单元格边距 108 缇，文件里的值不算数，它的段落格式和字符格式也不传给表格里的文字（改了名字的默认表格样式照常传）；不指向它的表格样式没有边距 | PDF | `normal_table_is_built_in_and_table_styles_format_paragraphs` |
| 页码域（`PAGE`、`NUMPAGES`）显示的是排版时算出的页码和页数，文件里存的结果只是上次保存时的值。域的指令部分（包括嵌套在指令里的域）不显示。`NUMPAGES` 不随节的页码格式变化，始终是阿拉伯数字 | WPS 的输出（`testdata/zh-open/11-header-footer`）；Word 的探针里没有域 | `page_number_fields_are_told_from_the_results_stored_for_them`、`each_page_gets_the_header_meant_for_it_with_its_number` |
| 节没有写明的页眉页脚沿用上一节的同一种（首页、偶数页、默认各自沿用）。"首页不同"按节设置，"奇偶页不同"按文档设置 | 首页、奇偶页的选用：PDF；沿用上一节：WPS 的输出 | `each_page_gets_the_header_meant_for_it_with_its_number`、`a_section_without_a_header_of_its_own_has_the_one_before_and_may_renumber` |
| 上下边距写成负数时，取绝对值，并且正文不再给更高的页眉（页脚）让位 | 规范；语料中 Word 文档的分页记录 | `reads_headers_and_footers_and_which_pages_they_are_for`、`headers_and_footers_stand_at_their_distances_and_push_the_body` |

## 已知但尚未实现

| 规则 | 说明 |
|---|---|
| 主题颜色优先于同一元素上缓存的颜色值，色调和阴影在亮度上计算 | 缓存值可能过期。已见到实例：标题颜色缓存为 `365F91`，按主题计算应为 `0F4761` |
| 目录和交叉引用域的结果不显示超链接字符样式的格式 | 影响所有带目录的文档；本条来自与 LibreOffice 的对比，尚未用 Word 材料确认 |
| 每个字符用哪个字体槽位：ASCII 用西文字体（包括汉字之间的空格和数字），汉字用东亚字体，开启复杂文字标记时整段用复杂文字字体 | 排版阶段需要 |
| 表格样式的条件格式（首行、首列、镶边等）和它们与段落样式的优先关系（`overrideTableStyleFontSizeAndJustification`） | 表格样式的基本叠加顺序和内置的 Normal Table 已实现，见结构一节 |
| 整数超出范围时不报错而是按 16 位回绕 | 仅影响异常文件 |
| zip 包内有重名条目时取目录中的第一个 | 通用 zip 库取最后一个，需要自有的读取层 |
| 未知元素的细节：未知元素里的段落并入外层段落，段落属性里的文字块提到段首等 | 仅见于探测文档 |

## LibreOffice 与 Word 不同的地方

用 LibreOffice 作参照时遇到的、它与 Word 读法不同的情况。基于 LibreOffice 做转换的方案会带有这些差异：

- 开关属性的值为空串或 `FALSE` 时，LibreOffice 读作关，Word 读作开。
- 域（如交叉引用）LibreOffice 会重新计算，目标不存在时显示错误文字；Word 打开时显示文件里保存的结果。
- 主文档根部是兼容性标记或正文元素时，LibreOffice 读不出内容，Word 可以。

排版方面的差异在做中文测试文档时发现，详见 [排版研究](layout.md)：

- 替换字体后，LibreOffice 按替换字体自身的度量算行高。Word 中宋体的行高是字号的 1.3 倍（实测），思源宋体按自身度量是 1.44 倍，在文档网格下每行多占一格，行距翻倍。
- 编号级别设置为"永不重新开始"时（规章里"条"的序号全文连续），LibreOffice 仍在每章重新编号。Word 的行为来自 docx4j 的测量记录。
- 文档要求给行尾空格加下划线时（合同里的填空线），LibreOffice 不画。
- `ordinal`、`cardinalText`、`decimalEnclosedParen` 三种数字格式，LibreOffice 的输出与规范描述不同。
- 网格下的固定值行距，LibreOffice 排成了网格行距。Word 的行为未经验证。
