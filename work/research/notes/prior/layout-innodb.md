---
title: InnoDB row formats and doublewrite
---
Clustered-index records with a 5-byte header, a reversed list of
variable-field lengths and a null bitmap, plus 6-byte DB_TRX_ID and
7-byte DB_ROLL_PTR. DYNAMIC moves long columns fully off-page behind a
20-byte pointer; COMPACT keeps a 768-byte prefix. Pages 4 to 64 KB,
16 KB default. Torn pages prevented by writing each page to doublewrite
files before its home location.
https://dev.mysql.com/doc/refman/8.4/en/innodb-row-format.html
https://dev.mysql.com/doc/refman/8.4/en/innodb-doublewrite-buffer.html
