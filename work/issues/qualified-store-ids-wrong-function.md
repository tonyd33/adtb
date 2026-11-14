---
created-at: 2026-11-13
status: open
kind: bug
priority: 330
summary: "a module-qualified store gives the view's catalog ids to a same-named function of the migration's module, or none"
tags:
  - class-identity-by-spelling
---

# A qualified `store` gives catalog ids to the wrong function

## Report

`module other` has `fn f () -> Set ({ a: Text })`; `module auth` has `fn f () -> Set Text`; a migration in `auth` runs `store other::f`. It applies, and `auth::f`, which is not stored, takes view relation, view type and view constructor ids, while `other::f`, which is stored, takes none. With no `f` in `auth`, the stored view takes no ids at all. Cause: ids/objects.rs `Decl::Store` looks up `values.get(&s.view.name.text)` and ids/named.rs `listed` uses `fns.get((m, vn))`, both dropping the qualifier, while the schema build resolves the qualified name with `lookup_value`. Same on main. Also: `listed` gives a Set-space id to a set declared on a stored view, which `objects` skips (it needs `ValueDef::Relation`), spending an id the built schema never holds. Found in the delta review of structure-lang-identity step 6a-2.

Direction: resolve the store's `QName` as the build does, in both `objects` and `listed`, from one helper; give a view's sets the same rule in both. Test: the migration above gives `other::f`'s view the ids.

Defect class: Catalog rows and identity by position or spelling.
