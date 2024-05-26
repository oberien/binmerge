# Binmerge

A binary diff / merge tool for large binaries.

![diff-view](/imgs/diff_viewer.png)

![apply confirmation dialog](/imgs/apply.png)

![quit confirmation dialog](/imgs/quit.png)

This was written out of a lack of binary diffing and merging tools that can handle large files like drive partitions.
It helped me restore a broken 2-disk RAID1.

### Features
* view the diff of very large files
* finds diffs in the background (currently at ~1GB/s per file)
* allows merging changes left, or right, or keep as-is

Not supported (yet?):
* jump to next/prev unmerged diff
* jump to next/prev merged diff
* jump to last position (Ctrl+O)
* make next/prev go relative to the screen position not to currently selected diff
* apply changes without closing the editor
* even faster diff algorithm
* diff algorithm with insert/delete
* show current percent-location in the file (e.g. `Position 0x12a0; 42%`)
* show diff file progress in percent (e.g. `Loading diffs, 4 so far (42% searched)`)
* open a file as write only when applying changes (vulnerable to a TOCTOU)
    * would allow viewing diff of readonly files
