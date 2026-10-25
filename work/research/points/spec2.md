---
extends: current
proposed: true
accept:
  spec/bad-input-source/empty: {title: Defined only for a statement over one source; otherwise source is "" and line is 0, reason: only a one-source statement has one failing record to name}
  spec/collation-fallback/root: {title: 'Root order, as CLDR falls back; sys::collations lists root and the tailored locales', reason: CLDR falls back to root for untailored locales}
  spec/collation-names/cldr: {title: 'The CLDR release of the tailoring data: de@cldr-49', reason: 'the CLDR data fixes the order, whatever library computes it'}
  spec/dump-key/passphrase: {title: 'A Text passphrase, turned into the key by Argon2id with a salt and parameters the manifest records', reason: mirrors the passphrase KEK; a restore needs only the passphrase}
  spec/dump-wrong-key/as-damaged: {title: The same failure as a damaged dump, reason: authenticated encryption cannot tell the two apart}
  spec/duration-form/largest-unit: {title: A DURATION literal in the largest unit that divides it exactly; zero is 0s, reason: 'short, exact, and read back by the literal grammar'}
  spec/password-param/text: {title: Text, reason: SCRAM normalizes passwords as text}
  spec/regex-flags/re2: {title: 'RE2''s i, m, s and U, anywhere in the pattern; x and R refused; . excludes a line feed unless s', reason: RE2 patterns keep working; x and R are not RE2}
  spec/rejected-source/unqualified: {title: The relation's unqualified name, reason: a rejects relation sits beside the load it serves}
  spec/temp-relations/generated-key/none: {title: 'No: no type names a temp relation, so its key is plain fields or its whole row', reason: no type names a temp relation}
  spec/temp-relations/names/distinct: {title: 'No: the name must not be a relation variable of the schema', reason: a name in ad hoc text means one relation}
  spec/temp-relations/redeclare/refused: {title: 'Refused, as any repeated declaration is', reason: as any repeated declaration is}
  spec/text-escapes/unicode-escape: {title: 'Printing writes a control character as \u{..}; reading accepts it raw or escaped', reason: printed text holds no invisible characters}
  spec/textual-forms/sortkey/hex: {title: '0x hex, as Bytes', reason: 'a SortKey is bytes, ordered as bytes'}
  spec/textual-forms/span/call: {title: 'span(a, b), the call that builds it', reason: span is the only form that builds a span}
  spec/totality/top-take/non-literal: {title: Partial unless the count is an integer literal, reason: 'a literal count is never negative, so never faults'}
  spec/unicode-version/one: {title: 'The database''s one recorded Unicode version, as every other text function', reason: one recorded Unicode version for every text function}
  spec/unique-names/every-ctor/unscoped: {title: 'As the unique with no in: the same scope, one canonical name', reason: one scope has one canonical name}
  spec/unique-names/named-set/ctors: {title: 'By the set''s constructors, in code-point order: in (Apple | Google)', reason: the name states the constructors the unique ranges over}
  spec/unique-names/unscoped/bare: {title: 'Without an in clause: m::r.unique(p1, ..., pn)', reason: the name is the declaration as written}
  spec/zone-edges/overflow: {title: They may fault ArithmeticOverflow, reason: as every other time operation at its range edge}
  spec/zone-names/exact: {title: Matched by exact spelling and kept as written, reason: the zone a client wrote comes back as written}
  spec/zone-overlap/earlier: {title: 'Not in the later half of an overlap: it gives the earlier instant, and the spec says so', reason: the round trip loses the later instant; state it}
  spec/zoned-edges/overflow: {title: 'It may fault ArithmeticOverflow, as in_zone and to_instant do', reason: as in_zone and to_instant do}
---
