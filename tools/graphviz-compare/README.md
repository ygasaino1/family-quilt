# Comparing our dot port with the real `dot`

`crates/layout/src/dotrank.rs` and `dotorder.rs` port Graphviz's ranking and ordering (see `UPSTREAM_PATHS`).
This folder is how we check that they still give **exactly** what `dot` gives, layer by layer and position by position.

1. Get the Graphviz source of the version you want to compare with (the release tarball from graphviz.org, the one
   named in `UPSTREAM`, or a newer one when the upstream-watch issue asks for a review) and build only `dot`:

       tar xzf graphviz-X.Y.Z.tar.gz && mkdir gv-build && cd gv-build
       ../graphviz-X.Y.Z/configure --prefix=$PWD/../gv-inst --disable-ltdl --without-x --without-expat \
         --without-zlib --without-gdk --without-pangocairo --without-webp --without-poppler --without-rsvg \
         --without-ghostscript --without-glut --without-qt --without-gtk --without-gts
       make -j8 && make install        # the program is gv-inst/bin/dot_builtins (or dot)

2. Build our side and run the comparison:

       cargo build -p family-layout --example dotrank_cmp
       python3 tools/graphviz-compare/compare.py --dot /path/to/dot_builtins --seeds 1-300

   `--file graph.dot` checks one graph (plain `"a"->"b";` lines, nodes numbered by first mention).

A newer `dot` that disagrees is the signal that the port needs review (or that a Graphviz change altered tie-breaking).
Last verified: Graphviz 16.1.2, Greek gods file, seeds 1-800 and a 5,600-node generated genealogy, all identical.
