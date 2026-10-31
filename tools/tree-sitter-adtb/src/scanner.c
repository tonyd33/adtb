// Emit the tokens the grammar cannot decide: keywords and operators by the token after them, the
// line and adjacency rules, and number, hash and bundle name tokens. Keyword and operator tokens
// are zero width and the plain token follows.

#include "tree_sitter/parser.h"

#include <stdbool.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#define WORD_MAX 64

// Keep the order equal to the grammar's externals.
enum Symbol {
  KW_ADD, KW_DROP, KW_RENAME, KW_REMOVE, KW_SPLIT, KW_MERGE, KW_NEST, KW_UNNEST, KW_MOVE, KW_ALTER,
  KW_REKEY, KW_DEFINE, KW_REDEFINE, KW_UNSTORE, KW_UNINSTALL, KW_INSTALL, KW_ENABLE, KW_DISABLE,
  KW_DEPRECATE, KW_REVERT, KW_SCRIPT, KW_ADOPT, KW_GRANT, KW_REVOKE, KW_STORE, KW_FEED,
  KW_AGGREGATE, KW_EXPLAIN, KW_PIN, KW_UNPIN, KW_CANCEL, KW_TERMINATE, KW_VACUUM, KW_VERIFY,
  KW_REINDEX, KW_COMPACT, KW_FINALIZE, KW_RETIRE, KW_SUPERSEDE, KW_EXPORT, KW_REGISTER, KW_APPLY,
  KW_PRINT, KW_ARCHIVE, KW_ROTATE, KW_DESTROY, KW_DUMP, KW_RESTORE, KW_BACKUP, KW_CREATE, KW_MAP,
  KW_UNMAP, KW_TAKE, KW_AFTER, KW_DESC, KW_ASC, KW_GENERATE, KW_UNIQUE, KW_INTERVAL, KW_INDEX,
  KW_VOLATILE, KW_DEFAULT, KW_COMPUTED, KW_LEASE, KW_PRIORITY, KW_BUDGET, KW_HIDE, KW_CATALOG,
  KW_DETERMINISTIC, KW_ARM, KW_JOINS, KW_BEFORE, KW_CARRY, KW_FILL, KW_RETRY, KW_READ_ONLY,
  KW_DEFERRABLE, KW_DURABILITY, KW_UNLESS, KW_REJECTS, KW_PINS, KW_SUMS, KW_ONLINE, KW_OFFLINE,
  KW_INCREMENTAL, KW_ENCRYPT, KW_WASM, KW_HOST, KW_ON_DELETE, KW_TO, KW_AT, KW_REJECT,
  KW_AT_RESTORE, KW_DEFAULT_GENERATE, KW_INSERT_FROM, KW_EXISTS_WHERE, KW_ORDER, KW_ANALYZE,
  TYPE_ARG, VERIFY_TARGET, ANON_FIRST, ANON_NEXT, PRELUDE_RECORD, CALL_LPAREN, TCALL_LPAREN,
  KEYS_LPAREN, GEN_COMMA, CTOR_DOT, TCALL_DOT, KEY_DOT, FIELD_CALL_BAD, ELISION_DOT, CTOR_LBRACE,
  MIDLINE_BAD, NEVER, END, UNION_OP, OR_OP, AND_OP, CMP_OP, ADD_OP, MUL_OP, FIELD_DOT, SELECTION_DOT,
  BACKLINK_DOT, ARM_BAR, QUALIFIER, TEXT_MORE, RECORD_ELISION, FIELD_COLON, WILDCARD, INT, DECIMAL, FLOAT, DURATION, MIGRATION, HASH,
  BUNDLE_NAME, FAMILY_NAME, REFUSED,
};

enum Kind {
  K_END, K_LOWER, K_RESERVED, K_UPPER, K_INT, K_DECIMAL, K_FLOAT, K_DURATION, K_TEXT, K_LPAREN,
  K_LBRACE, K_LBRACKET, K_BAR, K_OTHER,
};

typedef struct {
  enum Kind kind;
  bool spaced;
  char word[WORD_MAX];
} Next;

typedef bool (*Predicate)(const Next *next);

static bool in_list(const char *word, const char *const *list) {
  for (; *list != NULL; list++) {
    if (strcmp(word, *list) == 0) return true;
  }
  return false;
}

static const char *const RESERVED[] = {
  "and", "bundle", "by", "check", "constraint", "delete", "else", "exists", "fail", "false", "fix",
  "fn", "for", "from", "group", "if", "import", "in", "insert", "into", "key", "let", "match",
  "module", "not", "or", "order", "ordered", "query", "relation", "return", "schema", "select",
  "set", "skip", "temp", "then", "top", "transact", "true", "try", "txn", "type", "union", "update",
  "where", "with", NULL,
};

// Words that start a top level item, which end a type when first on their line.
static const char *const ITEM_WORDS[] = {
  "add", "drop", "rename", "remove", "split", "merge", "nest", "unnest", "move", "rekey", "define",
  "redefine", "store", "unstore", "enable", "disable", "deprecate", "revert", "script", "install",
  "uninstall", "adopt", "grant", "revoke", "feed", "aggregate", "explain", "create", "alter", "map",
  "unmap", NULL,
};

// Words that end a type because a clause follows it.
static const char *const CLAUSE_WORDS[] = {
  "on", "volatile", "unique", "interval", "index", "default", "computed", "fill", "generate", NULL,
};

static const char *const PRELUDE_TYPES[] = {
  "Set", "Key", "Group", "Decimal", "Tag", "I8", "I16", "I32", "I64", "F32", "F64", "Text", "Bytes",
  "SortKey", "Instant", "Date", "Time", "DateTime", "ZonedDateTime", "Duration", "Period", "Bool",
  "Option", "Result", "Rounding", "Overflow", "Fault", "Uuid", "Span", "Principal", "Origin",
  "Outcome", "Error", "TxnId", "Report", "Position", "RetryReason", "RejectReason", "FatalReason",
  "Notice", "RejectedRow", "Limit", NULL,
};

static bool next_word_in(const Next *next, const char *const *list) {
  return (next->kind == K_LOWER || next->kind == K_RESERVED) && in_list(next->word, list);
}

static bool next_lower(const Next *next) { return next->kind == K_LOWER; }
static bool next_int(const Next *next) { return next->kind == K_INT; }
static bool next_duration(const Next *next) { return next->kind == K_DURATION; }
static bool next_text(const Next *next) { return next->kind == K_TEXT; }
static bool next_lparen(const Next *next) { return next->kind == K_LPAREN; }
static bool next_lbrace(const Next *next) { return next->kind == K_LBRACE; }
static bool next_lower_or_upper(const Next *next) {
  return next->kind == K_LOWER || next->kind == K_UPPER;
}
static bool next_name(const Next *next) {
  return next->kind == K_LOWER || next->kind == K_UPPER || next->kind == K_RESERVED;
}

#define NEXT_WORDS(fn, ...)                                 \
  static bool fn(const Next *next) {                        \
    static const char *const list[] = {__VA_ARGS__, NULL};  \
    return next_word_in(next, list);                        \
  }

NEXT_WORDS(next_add, "module", "type", "relation", "set", "constructor", "field", "unique", "index",
           "interval", "default", "constraint", "check", "computed")
NEXT_WORDS(next_drop, "module", "type", "relation", "set", "field", "unique", "index", "interval",
           "default", "fn", "constraint", "check", "computed", "aggregate", "role")
NEXT_WORDS(next_rename, "type", "relation", "set", "constructor", "field")
NEXT_WORDS(next_constructor, "constructor")
NEXT_WORDS(next_alter, "field", "role")
NEXT_WORDS(next_field, "field")
NEXT_WORDS(next_relation, "relation")
NEXT_WORDS(next_define, "fn", "aggregate")
NEXT_WORDS(next_redefine, "fn", "computed")
NEXT_WORDS(next_tzdb, "tzdb")
NEXT_WORDS(next_plan, "plan")
NEXT_WORDS(next_call, "call")
NEXT_WORDS(next_session, "session")
NEXT_WORDS(next_upgrade, "upgrade")
NEXT_WORDS(next_bundle, "bundle")
NEXT_WORDS(next_migrations, "migrations")
NEXT_WORDS(next_schema, "schema")
NEXT_WORDS(next_audit, "audit")
NEXT_WORDS(next_key, "key")
NEXT_WORDS(next_to, "to")
NEXT_WORDS(next_from, "from")
NEXT_WORDS(next_role, "role")
NEXT_WORDS(next_peer, "peer")
NEXT_WORDS(next_index, "index")
NEXT_WORDS(next_into, "into")
NEXT_WORDS(next_delete, "delete")
NEXT_WORDS(next_commit_or_instant, "commit", "instant")
NEXT_WORDS(next_instant_or_position, "instant", "position")
NEXT_WORDS(next_write_keyword, "insert", "update", "delete", "fail", "skip", "if", "match")
NEXT_WORDS(next_expr_keyword, "true", "false", "from", "match", "fix", "fn", "exists", "top", "if",
           "key", "try", "not")

static bool next_write_start(const Next *next) {
  return next->kind == K_LPAREN || (next->kind == K_RESERVED && next_write_keyword(next));
}

static bool next_reject_operand(const Next *next) {
  if (!next->spaced) return false;
  switch (next->kind) {
    case K_LOWER: case K_UPPER: case K_INT: case K_DECIMAL: case K_FLOAT: case K_TEXT:
    case K_DURATION: case K_LPAREN: case K_LBRACE: case K_LBRACKET:
      return true;
    case K_RESERVED:
      return next_expr_keyword(next);
    default:
      return false;
  }
}

typedef struct {
  const char *word;
  enum Symbol symbol;
  Predicate holds;
} Contextual;

static const Contextual CONTEXTUAL[] = {
  {"add", KW_ADD, next_add}, {"drop", KW_DROP, next_drop}, {"rename", KW_RENAME, next_rename},
  {"remove", KW_REMOVE, next_constructor}, {"split", KW_SPLIT, next_constructor},
  {"merge", KW_MERGE, next_constructor}, {"nest", KW_NEST, next_constructor},
  {"unnest", KW_UNNEST, next_constructor}, {"move", KW_MOVE, next_field},
  {"alter", KW_ALTER, next_alter}, {"rekey", KW_REKEY, next_relation},
  {"define", KW_DEFINE, next_define}, {"redefine", KW_REDEFINE, next_redefine},
  {"unstore", KW_UNSTORE, next_lower}, {"uninstall", KW_UNINSTALL, next_lower},
  {"install", KW_INSTALL, next_lower}, {"enable", KW_ENABLE, next_lower_or_upper},
  {"disable", KW_DISABLE, next_lower_or_upper}, {"deprecate", KW_DEPRECATE, next_lower_or_upper},
  {"revert", KW_REVERT, next_int}, {"script", KW_SCRIPT, next_write_start},
  {"adopt", KW_ADOPT, next_tzdb}, {"grant", KW_GRANT, NULL}, {"revoke", KW_REVOKE, NULL},
  {"store", KW_STORE, next_lower}, {"feed", KW_FEED, next_lower},
  {"aggregate", KW_AGGREGATE, next_lower}, {"explain", KW_EXPLAIN, NULL},
  {"pin", KW_PIN, next_plan}, {"unpin", KW_UNPIN, next_plan}, {"cancel", KW_CANCEL, next_call},
  {"terminate", KW_TERMINATE, next_session}, {"vacuum", KW_VACUUM, next_lower},
  {"verify", KW_VERIFY, NULL}, {"reindex", KW_REINDEX, next_lower}, {"compact", KW_COMPACT, NULL},
  {"finalize", KW_FINALIZE, next_upgrade}, {"retire", KW_RETIRE, next_bundle},
  {"supersede", KW_SUPERSEDE, next_bundle}, {"export", KW_EXPORT, NULL},
  {"register", KW_REGISTER, next_bundle}, {"apply", KW_APPLY, next_migrations},
  {"print", KW_PRINT, next_schema}, {"archive", KW_ARCHIVE, next_audit},
  {"rotate", KW_ROTATE, next_key}, {"destroy", KW_DESTROY, next_key}, {"dump", KW_DUMP, next_to},
  {"restore", KW_RESTORE, next_from}, {"backup", KW_BACKUP, next_to},
  {"create", KW_CREATE, next_role}, {"map", KW_MAP, next_peer}, {"unmap", KW_UNMAP, next_peer},
  {"take", KW_TAKE, NULL}, {"after", KW_AFTER, NULL}, {"desc", KW_DESC, NULL},
  {"asc", KW_ASC, NULL}, {"generate", KW_GENERATE, NULL}, {"unique", KW_UNIQUE, next_lparen},
  {"interval", KW_INTERVAL, next_index}, {"index", KW_INDEX, next_lparen},
  {"volatile", KW_VOLATILE, NULL}, {"default", KW_DEFAULT, next_name},
  {"computed", KW_COMPUTED, next_lower}, {"lease", KW_LEASE, next_duration},
  {"priority", KW_PRIORITY, next_lower}, {"budget", KW_BUDGET, next_lbrace},
  {"hide", KW_HIDE, NULL}, {"catalog", KW_CATALOG, NULL},
  {"deterministic", KW_DETERMINISTIC, NULL}, {"arm", KW_ARM, next_lower},
  {"joins", KW_JOINS, NULL}, {"before", KW_BEFORE, NULL}, {"carry", KW_CARRY, NULL},
  {"fill", KW_FILL, NULL}, {"retry", KW_RETRY, next_int}, {"read_only", KW_READ_ONLY, NULL},
  {"deferrable", KW_DEFERRABLE, NULL}, {"durability", KW_DURABILITY, NULL},
  {"unless", KW_UNLESS, NULL}, {"rejects", KW_REJECTS, next_into}, {"pins", KW_PINS, next_int},
  {"sums", KW_SUMS, NULL}, {"online", KW_ONLINE, NULL}, {"offline", KW_OFFLINE, NULL},
  {"incremental", KW_INCREMENTAL, NULL}, {"encrypt", KW_ENCRYPT, NULL},
  {"wasm", KW_WASM, next_text}, {"host", KW_HOST, next_text}, {"on", KW_ON_DELETE, next_delete},
  {"to", KW_TO, NULL}, {"at", KW_AT_RESTORE, next_instant_or_position},
  {"generate", KW_DEFAULT_GENERATE, next_lower}, {"from", KW_INSERT_FROM, NULL},
  {"at", KW_AT, next_commit_or_instant}, {"reject", KW_REJECT, next_reject_operand},
  {"where", KW_EXISTS_WHERE, NULL}, {"order", KW_ORDER, NULL}, {"analyze", KW_ANALYZE, NULL},
  {"union", UNION_OP, NULL}, {"or", OR_OP, NULL}, {"and", AND_OP, NULL},
};

#define CONTEXTUAL_COUNT (sizeof(CONTEXTUAL) / sizeof(CONTEXTUAL[0]))

static void advance(TSLexer *lexer) { lexer->advance(lexer, false); }

static bool is_ident(int32_t c) {
  return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_';
}

static bool is_digit(int32_t c) { return c >= '0' && c <= '9'; }
static bool is_lower(int32_t c) { return c >= 'a' && c <= 'z'; }
static bool is_upper(int32_t c) { return c >= 'A' && c <= 'Z'; }

static bool is_hex(int32_t c) {
  return is_digit(c) || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F');
}

static bool is_space(int32_t c) { return c == ' ' || c == '\t' || c == '\r' || c == '\n'; }

static void read_word(TSLexer *lexer, char *word) {
  size_t n = 0;
  while (is_ident(lexer->lookahead) && !lexer->eof(lexer)) {
    if (n + 1 < WORD_MAX) word[n++] = (char)lexer->lookahead;
    advance(lexer);
  }
  word[n] = '\0';
}

static bool read_unit(TSLexer *lexer) {
  char word[WORD_MAX];
  read_word(lexer, word);
  static const char *const units[] = {"us", "ms", "min", "s", "h", "d", NULL};
  return in_list(word, units);
}

// Advance over a number and return its kind.
// Postconditions:
// - With `mark`, the token end lies after the number.
static enum Kind read_number(TSLexer *lexer, bool mark) {
  while (is_digit(lexer->lookahead)) advance(lexer);
  if (mark) lexer->mark_end(lexer);
  enum Kind kind = K_INT;
  if (lexer->lookahead == '.') {
    advance(lexer);
    if (!is_digit(lexer->lookahead)) return K_INT;
    while (is_digit(lexer->lookahead)) advance(lexer);
    if (mark) lexer->mark_end(lexer);
    kind = K_DECIMAL;
  }
  if (lexer->lookahead == 'e' || lexer->lookahead == 'E') {
    advance(lexer);
    if (lexer->lookahead == '+' || lexer->lookahead == '-') advance(lexer);
    if (!is_digit(lexer->lookahead)) return kind;
    while (is_digit(lexer->lookahead)) advance(lexer);
    if (mark) lexer->mark_end(lexer);
    return K_FLOAT;
  }
  if (kind == K_INT && is_ident(lexer->lookahead) && read_unit(lexer)) {
    if (mark) lexer->mark_end(lexer);
    return K_DURATION;
  }
  return kind;
}

enum Gap { GAP_NONE, GAP_PASSED, GAP_MINUS };

// Advance over whitespace and comments.
// Returns `GAP_MINUS` when a lone `-` follows, which is left consumed.
static enum Gap pass_gap(TSLexer *lexer) {
  enum Gap gap = GAP_NONE;
  for (;;) {
    if (is_space(lexer->lookahead)) {
      advance(lexer);
      gap = GAP_PASSED;
    } else if (lexer->lookahead == '-') {
      advance(lexer);
      if (lexer->lookahead != '-') return GAP_MINUS;
      while (lexer->lookahead != '\n' && !lexer->eof(lexer)) advance(lexer);
      gap = GAP_PASSED;
    } else {
      return gap;
    }
  }
}

static void read_next(TSLexer *lexer, Next *next) {
  next->word[0] = '\0';
  enum Gap gap = pass_gap(lexer);
  next->spaced = gap != GAP_NONE;
  int32_t c = lexer->lookahead;
  if (gap == GAP_MINUS) {
    next->kind = K_OTHER;
  } else if (lexer->eof(lexer)) {
    next->kind = K_END;
  } else if (is_lower(c)) {
    read_word(lexer, next->word);
    next->kind = in_list(next->word, RESERVED) ? K_RESERVED : K_LOWER;
  } else if (is_upper(c)) {
    read_word(lexer, next->word);
    next->kind = K_UPPER;
  } else if (is_digit(c)) {
    next->kind = read_number(lexer, false);
  } else if (c == '"') {
    next->kind = K_TEXT;
  } else if (c == '(') {
    next->kind = K_LPAREN;
  } else if (c == '{') {
    next->kind = K_LBRACE;
  } else if (c == '[') {
    next->kind = K_LBRACKET;
  } else if (c == '|') {
    next->kind = K_BAR;
  } else {
    next->kind = K_OTHER;
  }
}

static bool emit(TSLexer *lexer, enum Symbol symbol) {
  lexer->result_symbol = symbol;
  return true;
}

// Emit a token no state accepts, at least one character wide.
static bool refuse(TSLexer *lexer) {
  if (!lexer->eof(lexer)) advance(lexer);
  lexer->mark_end(lexer);
  return emit(lexer, REFUSED);
}

// Returns whether the parenthesis at the lexer closes before `.`, a word and an adjacent `(`.
static bool paren_is_type_call(TSLexer *lexer) {
  int depth = 1;
  while (depth > 0) {
    if (lexer->eof(lexer)) return false;
    int32_t c = lexer->lookahead;
    if (c == '"') {
      advance(lexer);
      while (lexer->lookahead != '"') {
        if (lexer->eof(lexer) || lexer->lookahead == '\n') return false;
        if (lexer->lookahead == '\\') advance(lexer);
        advance(lexer);
      }
      advance(lexer);
    } else if (c == '-') {
      advance(lexer);
      if (lexer->lookahead == '-') {
        while (lexer->lookahead != '\n' && !lexer->eof(lexer)) advance(lexer);
      }
    } else {
      if (c == '(') depth++;
      if (c == ')') depth--;
      advance(lexer);
    }
  }
  if (pass_gap(lexer) == GAP_MINUS || lexer->lookahead != '.') return false;
  advance(lexer);
  if (pass_gap(lexer) == GAP_MINUS || !is_lower(lexer->lookahead)) return false;
  char word[WORD_MAX];
  read_word(lexer, word);
  return !in_list(word, RESERVED) && lexer->lookahead == '(';
}

static bool scan_hash(TSLexer *lexer, const bool *valid) {
  int digits = 0;
  while (is_hex(lexer->lookahead) && digits <= 64) {
    advance(lexer);
    digits++;
  }
  if (lexer->lookahead == '.') {
    for (int i = 0; i < 3; i++) {
      if (lexer->lookahead != '.') return false;
      advance(lexer);
    }
  } else if (digits != 64) {
    return false;
  }
  lexer->mark_end(lexer);
  return valid[HASH] && emit(lexer, HASH);
}

static bool version_shape(const char *run, size_t n, size_t *dash_at) {
  size_t dash = n;
  for (size_t i = n; i > 0; i--) {
    if (run[i - 1] == '-') {
      dash = i - 1;
      break;
    }
  }
  if (dash == n) return false;
  *dash_at = dash;
  if (dash + 1 >= n || !is_digit(run[dash + 1])) return false;
  for (size_t i = dash + 1; i < n; i++) {
    if (!is_digit(run[i]) && run[i] != '.') return false;
  }
  for (size_t i = 0; i < dash; i++) {
    if (run[i] == '.') return false;
  }
  return is_lower(run[0]);
}

static bool scan_bundle_name(TSLexer *lexer, const bool *valid, const char *first_word) {
  char run[256];
  size_t n = strlen(first_word);
  if (n >= sizeof(run)) return false;
  memcpy(run, first_word, n);
  size_t end = n;
  lexer->mark_end(lexer);
  while (is_lower(lexer->lookahead) || is_digit(lexer->lookahead) || lexer->lookahead == '_' ||
         lexer->lookahead == '-' || lexer->lookahead == '.') {
    if (n + 1 >= sizeof(run)) return false;
    run[n++] = (char)lexer->lookahead;
    char c = (char)lexer->lookahead;
    advance(lexer);
    if (c != '.') {
      end = n;
      lexer->mark_end(lexer);
    }
  }
  size_t dash;
  if (valid[BUNDLE_NAME] && version_shape(run, end, &dash)) return emit(lexer, BUNDLE_NAME);
  if (!valid[FAMILY_NAME]) return false;
  for (size_t i = 0; i < end; i++) {
    if (run[i] == '.') return false;
  }
  return !is_upper(lexer->lookahead) && emit(lexer, FAMILY_NAME);
}

// Close the expression at the lexer, or refuse a `bare` query that does not start its line.
static bool end_or_bare(TSLexer *lexer, const bool *valid, bool first_on_line, bool bare) {
  if (valid[END]) return emit(lexer, END);
  return bare && valid[MIDLINE_BAD] && !first_on_line && refuse(lexer);
}

static bool scan_upper(TSLexer *lexer, const bool *valid, bool first_on_line) {
  char word[WORD_MAX];
  read_word(lexer, word);
  bool called = lexer->lookahead == '(';
  if (valid[TYPE_ARG] && !called) return emit(lexer, TYPE_ARG);
  bool prelude = in_list(word, PRELUDE_TYPES);
  if (valid[ANON_NEXT] && !prelude) return emit(lexer, ANON_NEXT);
  if (valid[ANON_FIRST] || valid[PRELUDE_RECORD]) {
    Next next;
    read_next(lexer, &next);
    if (valid[ANON_FIRST] && !prelude && (next.kind == K_LBRACE || next.kind == K_BAR)) {
      return emit(lexer, ANON_FIRST);
    }
    if (valid[PRELUDE_RECORD] && prelude && strcmp(word, "Key") != 0 && next.kind == K_LBRACE) {
      return emit(lexer, PRELUDE_RECORD);
    }
  }
  return end_or_bare(lexer, valid, first_on_line, true);
}

static bool scan_lower(TSLexer *lexer, const bool *valid, bool first_on_line) {
  char word[WORD_MAX];
  read_word(lexer, word);
  bool reserved = in_list(word, RESERVED);
  bool item_word = first_on_line && in_list(word, ITEM_WORDS);
  if (!reserved) {
    bool called = lexer->lookahead == '(';
    bool grant_to = valid[KW_TO] && strcmp(word, "to") == 0;
    if (valid[TYPE_ARG] && !called && !in_list(word, CLAUSE_WORDS) && !item_word && !grant_to) {
      return emit(lexer, TYPE_ARG);
    }
    if (valid[VERIFY_TARGET] && !item_word) return emit(lexer, VERIFY_TARGET);
    if (valid[KW_ANALYZE] && strcmp(word, "analyze") == 0) return emit(lexer, KW_ANALYZE);
    if (valid[BUNDLE_NAME] || valid[FAMILY_NAME]) return scan_bundle_name(lexer, valid, word);
  }
  Next next;
  bool have_next = false;
  for (size_t i = 0; i < CONTEXTUAL_COUNT; i++) {
    const Contextual *entry = &CONTEXTUAL[i];
    if (!valid[entry->symbol] || strcmp(word, entry->word) != 0) continue;
    if ((entry->symbol == KW_AT || entry->symbol == KW_REJECT) && valid[MIDLINE_BAD] &&
        !first_on_line) {
      return refuse(lexer);
    }
    if (entry->holds != NULL) {
      if (!have_next) {
        read_next(lexer, &next);
        have_next = true;
      }
      if (!entry->holds(&next)) continue;
    }
    return emit(lexer, entry->symbol);
  }
  return end_or_bare(lexer, valid, first_on_line, !reserved);
}

static bool scan_number(TSLexer *lexer, const bool *valid, bool first_on_line) {
  if (valid[TYPE_ARG]) {
    if (read_number(lexer, false) == K_INT) return emit(lexer, TYPE_ARG);
    return end_or_bare(lexer, valid, first_on_line, true);
  }
  if (end_or_bare(lexer, valid, first_on_line, true)) return true;
  if (valid[MIGRATION]) {
    int digits = 0;
    while (is_digit(lexer->lookahead)) {
      advance(lexer);
      digits++;
    }
    lexer->mark_end(lexer);
    return digits >= 4 && emit(lexer, MIGRATION);
  }
  switch (read_number(lexer, true)) {
    case K_INT: return valid[INT] && emit(lexer, INT);
    case K_DECIMAL: return valid[DECIMAL] && emit(lexer, DECIMAL);
    case K_FLOAT: return valid[FLOAT] && emit(lexer, FLOAT);
    case K_DURATION: return valid[DURATION] && emit(lexer, DURATION);
    default: return false;
  }
}

static bool scan_dot(TSLexer *lexer, const bool *valid, bool first_on_line) {
  advance(lexer);
  if (lexer->lookahead == '<') {
    if (valid[BACKLINK_DOT]) return emit(lexer, BACKLINK_DOT);
    return end_or_bare(lexer, valid, first_on_line, false);
  }
  if (pass_gap(lexer) == GAP_MINUS) return end_or_bare(lexer, valid, first_on_line, false);
  int32_t c = lexer->lookahead;
  if (c == '.') {
    if (valid[RECORD_ELISION]) return emit(lexer, RECORD_ELISION);
    advance(lexer);
    if (pass_gap(lexer) != GAP_MINUS && lexer->lookahead == '.') {
      if (valid[END]) return emit(lexer, END);
      if (valid[MIDLINE_BAD] && !first_on_line) return refuse(lexer);
      return valid[ELISION_DOT] && emit(lexer, ELISION_DOT);
    }
    return end_or_bare(lexer, valid, first_on_line, false);
  }
  if (is_upper(c)) {
    if (valid[CTOR_DOT]) return emit(lexer, CTOR_DOT);
    if (valid[SELECTION_DOT]) return emit(lexer, SELECTION_DOT);
    return end_or_bare(lexer, valid, first_on_line, false);
  }
  if (c == '(') {
    advance(lexer);
    if (pass_gap(lexer) != GAP_MINUS && is_upper(lexer->lookahead) && valid[SELECTION_DOT]) return emit(lexer, SELECTION_DOT);
    return end_or_bare(lexer, valid, first_on_line, false);
  }
  if (!is_lower(c)) return end_or_bare(lexer, valid, first_on_line, false);
  char word[WORD_MAX];
  read_word(lexer, word);
  if (lexer->lookahead == '(') {
    bool reserved = in_list(word, RESERVED);
    if (valid[TCALL_DOT] && !reserved) return emit(lexer, TCALL_DOT);
    if (valid[KEY_DOT] && strcmp(word, "key") == 0) return emit(lexer, KEY_DOT);
    if (valid[FIELD_CALL_BAD]) return refuse(lexer);
  }
  if (valid[FIELD_DOT]) return emit(lexer, FIELD_DOT);
  return end_or_bare(lexer, valid, first_on_line, false);
}

static bool scan_minus(TSLexer *lexer, const bool *valid, bool first_on_line) {
  if (valid[ADD_OP]) return emit(lexer, ADD_OP);
  return end_or_bare(lexer, valid, first_on_line, true);
}

static bool scan_symbol(TSLexer *lexer, const bool *valid, bool first_on_line, bool spaced) {
  int32_t c = lexer->lookahead;
  switch (c) {
    case '(':
      if (valid[TYPE_ARG]) return emit(lexer, TYPE_ARG);
      if (valid[CALL_LPAREN] && !spaced) return emit(lexer, CALL_LPAREN);
      if (valid[KEYS_LPAREN]) return emit(lexer, KEYS_LPAREN);
      if (valid[END]) return emit(lexer, END);
      if (valid[TCALL_LPAREN]) {
        advance(lexer);
        return paren_is_type_call(lexer) && emit(lexer, TCALL_LPAREN);
      }
      return false;
    case '{':
      if (valid[TYPE_ARG]) return emit(lexer, TYPE_ARG);
      if (valid[CTOR_LBRACE]) return emit(lexer, CTOR_LBRACE);
      return end_or_bare(lexer, valid, first_on_line, true);
    case '"':
      if (valid[TEXT_MORE]) return emit(lexer, TEXT_MORE);
      return end_or_bare(lexer, valid, first_on_line, true);
    case '[':
      return end_or_bare(lexer, valid, first_on_line, true);
    case '+':
      if (valid[ADD_OP]) return emit(lexer, ADD_OP);
      return end_or_bare(lexer, valid, first_on_line, false);
    case '*':
    case '/':
    case '%':
      if (valid[MUL_OP]) return emit(lexer, MUL_OP);
      return end_or_bare(lexer, valid, first_on_line, false);
    case '<':
    case '>':
      if (valid[CMP_OP]) return emit(lexer, CMP_OP);
      return end_or_bare(lexer, valid, first_on_line, false);
    case '=':
      advance(lexer);
      if (lexer->lookahead == '=' && valid[CMP_OP]) return emit(lexer, CMP_OP);
      return end_or_bare(lexer, valid, first_on_line, false);
    case '!':
      advance(lexer);
      if (lexer->lookahead == '=' && valid[CMP_OP]) return emit(lexer, CMP_OP);
      return end_or_bare(lexer, valid, first_on_line, false);
    case '|':
      if (valid[ARM_BAR]) return emit(lexer, ARM_BAR);
      return end_or_bare(lexer, valid, first_on_line, false);
    case ':':
      advance(lexer);
      if (lexer->lookahead == ':' && valid[QUALIFIER]) return emit(lexer, QUALIFIER);
      if (lexer->lookahead != ':' && valid[FIELD_COLON]) return emit(lexer, FIELD_COLON);
      return end_or_bare(lexer, valid, first_on_line, false);
    case ',':
      if (valid[GEN_COMMA]) {
        advance(lexer);
        Next next;
        read_next(lexer, &next);
        if (next.kind == K_LOWER) {
          Next after;
          read_next(lexer, &after);
          if (after.kind == K_RESERVED && strcmp(after.word, "in") == 0) {
            return emit(lexer, GEN_COMMA);
          }
        }
      }
      return end_or_bare(lexer, valid, first_on_line, false);
    default:
      return end_or_bare(lexer, valid, first_on_line, false);
  }
}

// `line_start` holds that the token at `column` is first on its line, across zero width tokens.
typedef struct {
  uint32_t column;
  bool seen;
  bool line_start;
} State;

static bool scan(State *state, TSLexer *lexer, const bool *valid) {
  if (valid[NEVER]) return false;
  if (valid[HASH]) return scan_hash(lexer, valid);

  bool first_on_line = !state->seen && lexer->get_column(lexer) == 0;
  bool spaced = false;
  while (is_space(lexer->lookahead)) {
    if (lexer->lookahead == '\n') first_on_line = true;
    lexer->advance(lexer, true);
    spaced = true;
  }
  if (!spaced && state->line_start) first_on_line = lexer->get_column(lexer) == state->column;
  uint32_t column = first_on_line ? lexer->get_column(lexer) : 0;
  lexer->mark_end(lexer);

  bool found;
  int32_t c = lexer->lookahead;
  if (lexer->eof(lexer)) {
    found = valid[END] && emit(lexer, END);
  } else if (is_lower(c)) {
    found = scan_lower(lexer, valid, first_on_line);
  } else if (is_upper(c)) {
    found = scan_upper(lexer, valid, first_on_line);
  } else if (is_digit(c)) {
    found = scan_number(lexer, valid, first_on_line);
  } else if (c == '.') {
    found = scan_dot(lexer, valid, first_on_line);
  } else if (c == '_') {
    advance(lexer);
    if (is_ident(lexer->lookahead)) return false;
    lexer->mark_end(lexer);
    found = valid[WILDCARD] && emit(lexer, WILDCARD);
  } else if (c == '-') {
    advance(lexer);
    if (lexer->lookahead == '-') return false;
    if (lexer->lookahead == '>') {
      found = end_or_bare(lexer, valid, first_on_line, false);
    } else {
      found = scan_minus(lexer, valid, first_on_line);
    }
  } else {
    found = scan_symbol(lexer, valid, first_on_line, spaced);
  }
  if (found) {
    state->seen = true;
    state->line_start = first_on_line;
    state->column = column;
  }
  return found;
}

void *tree_sitter_adtb_external_scanner_create(void) { return calloc(1, sizeof(State)); }
void tree_sitter_adtb_external_scanner_destroy(void *payload) { free(payload); }
unsigned tree_sitter_adtb_external_scanner_serialize(void *payload, char *buffer) {
  memcpy(buffer, payload, sizeof(State));
  return sizeof(State);
}
void tree_sitter_adtb_external_scanner_deserialize(void *payload, const char *buffer, unsigned length) {
  memset(payload, 0, sizeof(State));
  if (length == sizeof(State)) memcpy(payload, buffer, sizeof(State));
}
bool tree_sitter_adtb_external_scanner_scan(void *payload, TSLexer *lexer, const bool *valid_symbols) {
  return scan(payload, lexer, valid_symbols);
}
