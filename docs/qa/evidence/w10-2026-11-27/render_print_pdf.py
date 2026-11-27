import os, pymupdf
pdf = r'D:\VSCode\Veil\docs\qa\evidence\w10-2026-11-27\print-to-pdf.pdf'
out = r'D:\VSCode\Veil\docs\qa\evidence\w10-2026-11-27'
doc = pymupdf.open(pdf)
print('pymupdf', pymupdf.__version__, 'pages', doc.page_count)
for i, page in enumerate(doc):
    t = page.get_text()
    cjk = sum(1 for c in t if '\u4e00' <= c <= '\u9fff')
    fonts = sorted({f[3] for f in page.get_fonts()})
    print(f'page {i+1}: chars={len(t)} cjk={cjk} fonts={fonts}')
    page.get_pixmap(matrix=pymupdf.Matrix(2,2)).save(os.path.join(out, f'print-pdf-mupdf-p{i+1}.png'))
print('head', repr(doc[0].get_text()[:100]))
