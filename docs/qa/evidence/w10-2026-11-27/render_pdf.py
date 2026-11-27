import os
import pymupdf

pdf = r"D:\VSCode\Veil\docs\qa\evidence\w10-2026-11-27\veil-export.pdf"
out = r"D:\VSCode\Veil\docs\qa\evidence\w10-2026-11-27"
doc = pymupdf.open(pdf)
print("pymupdf", pymupdf.__version__, "pages", doc.page_count)
for i, page in enumerate(doc):
    text = page.get_text()
    cjk = sum(1 for c in text if "\u4e00" <= c <= "\u9fff")
    fonts = sorted({f[3] for f in page.get_fonts()})
    print(f"page {i + 1}: chars={len(text)} cjk={cjk} fonts={fonts}")
    pix = page.get_pixmap(matrix=pymupdf.Matrix(2, 2))
    pix.save(os.path.join(out, f"pdf-mupdf-p{i + 1}.png"))
print("page1 text head:", repr(doc[0].get_text()[:160]))
