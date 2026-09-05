import pathlib, sys
cur, new, today = sys.argv[1], sys.argv[2], sys.argv[3]
c = pathlib.Path("Cargo.toml")
c.write_text(c.read_text().replace('version = "%s"' % cur, 'version = "%s"' % new, 1))
g = pathlib.Path("CHANGELOG.md")
g.write_text(g.read_text().replace("## 미출시\n", "## 미출시\n\n## %s (%s)\n" % (new, today), 1))
