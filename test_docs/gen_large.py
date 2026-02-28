#!/usr/bin/env python3
"""Generate a large LaTeX document for benchmarking"""

import sys
import random
import string

def random_word(min_len=3, max_len=12):
    return ''.join(random.choices(string.ascii_lowercase, k=random.randint(min_len, max_len)))

def random_sentence(min_words=5, max_words=25):
    words = [random_word() for _ in range(random.randint(min_words, max_words))]
    words[0] = words[0].capitalize()
    return ' '.join(words) + '.'

def random_paragraph(min_sentences=3, max_sentences=10):
    return ' '.join(random_sentence() for _ in range(random.randint(min_sentences, max_sentences)))

target_pages = int(sys.argv[1]) if len(sys.argv) > 1 else 100

print(r"""\documentclass[12pt,a4paper]{article}
\usepackage[margin=1in]{geometry}
\usepackage{amsmath}

\title{Large Scale Benchmark Document}
\author{SonicSpeed Benchmark Suite}
\date{February 2026}

\begin{document}

\maketitle

\begin{abstract}
""")
print(random_paragraph(5, 8))
print(r"\end{abstract}")
print()
print(r"\tableofcontents")
print()

# Each page is roughly 40-50 lines of text
sections_needed = target_pages // 3

for sec in range(sections_needed):
    print(f"\n\\section{{{random_word(5,15).capitalize()} {random_word(3,10).capitalize()} {random_word(4,12).capitalize()}}}\n")

    # 2-4 paragraphs per section
    for _ in range(random.randint(2, 4)):
        print(random_paragraph(4, 8))
        print()

    # Add some math
    if random.random() < 0.5:
        print(r"$$" + f"x_{{{sec}}} = \\frac{{{random.randint(1,100)}}}{{{random.randint(1,50)}}}" + r"$$")
        print()

    # Subsections
    for sub in range(random.randint(1, 3)):
        print(f"\\subsection{{{random_word(4,10).capitalize()} {random_word(3,8).capitalize()}}}\n")

        for _ in range(random.randint(2, 3)):
            print(random_paragraph(3, 7))
            print()

        # Add lists sometimes
        if random.random() < 0.3:
            print(r"\begin{itemize}")
            for _ in range(random.randint(3, 6)):
                print(f"\\item {random_sentence(3, 12)}")
            print(r"\end{itemize}")
            print()

        # Add tables sometimes
        if random.random() < 0.2:
            cols = random.randint(3, 5)
            rows = random.randint(3, 8)
            col_spec = '|'.join(['c'] * cols)
            print(r"\begin{table}[htbp]")
            print(r"\centering")
            print(f"\\caption{{{random_sentence(3, 6)}}}")
            print(f"\\begin{{tabular}}{{|{col_spec}|}}")
            print(r"\hline")
            # Header
            headers = [random_word(3,8).capitalize() for _ in range(cols)]
            print(' & '.join(headers) + r' \\')
            print(r"\hline")
            for _ in range(rows):
                cells = [str(random.randint(1, 1000)) if random.random() < 0.5 else random_word(3,8) for _ in range(cols)]
                print(' & '.join(cells) + r' \\')
            print(r"\hline")
            print(r"\end{tabular}")
            print(r"\end{table}")
            print()

        # Code blocks sometimes
        if random.random() < 0.15:
            print(r"\begin{verbatim}")
            for _ in range(random.randint(3, 10)):
                indent = "    " * random.randint(0, 3)
                print(f"{indent}{random_word()} = {random_word()}({random.randint(0,100)});")
            print(r"\end{verbatim}")
            print()

    # Display math
    if random.random() < 0.3:
        print(r"\begin{equation}")
        print(f"\\sum_{{i=1}}^{{n}} {random_word(1,3)}_i = \\int_0^{{\\infty}} f(x) dx")
        print(r"\end{equation}")
        print()

print(r"""
\section{Conclusion}
""")
print(random_paragraph(5, 10))
print()
print(r"\end{document}")
