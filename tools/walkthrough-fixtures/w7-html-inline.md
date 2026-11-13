# Wave 7 行内 HTML 真机验收（12 标签 × 7 上下文）

12 标签：kbd / sub / sup / br / mark / abbr / img / a / code / pre / details / summary
7 上下文：正文 / 标题 / 列表 / 引用 / 表格 / 链接文本 / 图片 alt

## 1. 正文段落

段落内：<kbd>Ctrl</kbd>、H<sub>2</sub>O、x<sup>2</sup>、第一行<br>第二行、<mark>高亮</mark>、<abbr title="HyperText Markup Language">HTML</abbr>、<img src="veil-logo.png" alt="标志">、<a href="https://example.com">链接</a>、<code>行内代码</code>、<pre>预格式</pre>、<details><summary>摘要</summary>折叠正文</details>。

### 1.1 kbd 细节

按 <kbd>Ctrl</kbd> + <kbd>Shift</kbd> + <kbd>P</kbd> 打开命令面板；单个 <kbd>K</kbd> 也要有描边圆角。

### 1.2 sub / sup 细节

化学式 H<sub>2</sub>SO<sub>4</sub>、数学式 E = mc<sup>2</sup>、上标连续 a<sup>1</sup><sup>2</sup><sup>3</sup>。

### 1.3 mark / abbr 细节

这里 <mark>黄色标记</mark> 与 <abbr title="Application Programming Interface">API</abbr> 缩写；连续 <mark>多字标记文本</mark> 看底色范围。

### 1.4 code / pre 细节

行内 <code>cargo build --release</code> 与块内 <pre>fn main() { 中文 }</pre>。

## 2. ATX 标题

## 2.1 标题内 <kbd>K</kbd> 与 H<sub>2</sub>O 与 x<sup>2</sup>

### 2.2 标题内 <mark>标记</mark> 与 <abbr title="abbr in heading">AB</abbr> 与 <code>code</code>

#### 2.3 标题内 <a href="https://example.com">锚点</a> 与 <img src="veil-logo.png" alt="图">

## 3. 列表

- 无序：<kbd>Ctrl</kbd> + <kbd>A</kbd>，H<sub>2</sub>O，x<sup>2</sup>，<mark>高</mark>
- 无序：第一行<br>第二行，<abbr title="abbr in list">AB</abbr>，<a href="https://example.com">链接</a>
- 无序：<img src="veil-logo.png" alt="标志">，<code>c</code>，<pre>p</pre>
- 无序：<details><summary>列表内摘要</summary>列表内折叠正文</details>

1. 有序：<kbd>Enter</kbd> 与 <sup>1</sup>
2. 有序：<mark>标记</mark> 与 <code>code</code>

## 4. 引用块

> 引用内：<kbd>Ctrl</kbd> + <kbd>C</kbd>，H<sub>2</sub>O，x<sup>2</sup>，<mark>高亮</mark>。
>
> 引用内第二段：第一行<br>第二行、<abbr title="abbr in quote">AB</abbr>、<a href="https://example.com">链接</a>、<code>c</code>、<pre>p</pre>。
>
> 引用内折叠：<details><summary>引用摘要</summary>引用折叠正文</details>

## 5. 表格单元格

| 标签 | 单元格内容 |
| --- | --- |
| kbd | <kbd>Ctrl</kbd>+<kbd>A</kbd> |
| sub/sup | H<sub>2</sub>O 与 x<sup>2</sup> |
| br | 第一行<br>第二行 |
| mark | <mark>高亮</mark> |
| abbr | <abbr title="abbr in table">AB</abbr> |
| a | <a href="https://example.com">链接</a> |
| code | <code>c</code> |
| pre | <pre>p</pre> |
| details | <details><summary>表内摘要</summary>表内折叠</details> |
| img | <img src="veil-logo.png" alt="标志"> |

## 6. 链接文本

[按 <kbd>Ctrl</kbd>+<kbd>A</kbd> 全选](https://example.com/a) —— 链接可点，文本标签生效。

[带 <sup>1</sup> 与 <mark>标记</mark> 与 <code>code</code> 的链接](https://example.com/b)。

## 7. 图片 alt

![<sup>1</sup> 与 <kbd>K</kbd> 的 alt](veil-missing.png)

图片不存在时降级为占位 + alt 文本。

## 8. 安全与降级

- 白名单外标签：<div>div</div> 与 <span>span</span> 应原样显示。
- 属性过滤：<kbd onclick="alert(1)" style="color:red">K</kbd> 只保留标签本身。
- javascript 链接：<a href="javascript:alert(1)">危险链接</a> 不应可跳转。
- 远程图片：<img src="https://example.com/remote.png" alt="远程"> 不发网络请求，占位 + alt。
- 未闭合标签：<kbd 未闭合 应原样显示。
- 交叉标签：<kbd><mark>交叉</kbd></mark> 应原样显示。
