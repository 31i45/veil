# 验证系统打印件中的短代码块是否保持在同一页（Task 5.3 / 6.3 证据脚本）。
# 依赖：pip install pymupdf
import os
import pymupdf

PDF = r'D:\VSCode\Veil\docs\qa\evidence\w10-2026-11-27\print-to-pdf.pdf'
OUT = r'D:\VSCode\Veil\docs\qa\evidence\w10-2026-11-27'
LINES = [
    'fn main() {',
    'let theme = veil_ui::Theme::dark();',
    'println!("dark bg =',
    '// 中文注释：代码块底色、行高与等宽字体在 PDF 中应保持可读。',
    'for i in 0..3 {',
    'println!("line {i}");',
]

doc = pymupdf.open(PDF)
print('pages', doc.page_count, 'bytes', os.path.getsize(PDF))
line_pages = {}
for i, page in enumerate(doc):
    text = page.get_text()
    hits = [j for j, line in enumerate(LINES) if line in text]
    print(f'page {i + 1}: chars={len(text)} code_lines={hits}')
    for j in hits:
        line_pages.setdefault(j, []).append(i + 1)
    page.get_pixmap(matrix=pymupdf.Matrix(2, 2)).save(os.path.join(OUT, f'print-pdf-mupdf-p{i + 1}.png'))

missing = [j for j in range(len(LINES)) if j not in line_pages]
duplicated = {j: pages for j, pages in line_pages.items() if len(pages) > 1}
pages = {p[0] for p in line_pages.values() if len(p) == 1}
print('line_pages', line_pages)
if not missing and not duplicated and len(pages) == 1:
    print('PASS_CODE_BLOCK_ON_ONE_PAGE page=', next(iter(pages)))
else:
    print('FAIL_CODE_BLOCK_SPLIT_OR_MISSING', 'missing=', missing, 'duplicated=', duplicated)
