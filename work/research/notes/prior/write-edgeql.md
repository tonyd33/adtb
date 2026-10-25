---
title: EdgeQL insert, update and unless conflict
---
`insert` and `update` are expressions returning objects, nestable as
link values and wrapped by `select` for shapes. Upsert is
`insert ... unless conflict on .k else (update ...)`, against an
`exclusive` constraint. Bulk is `for x in ... union (insert ...)`.
Link policies include `deferred restrict`.
https://docs.geldata.com/reference/edgeql/insert
https://docs.geldata.com/reference/datamodel/links
