use crate::span::Span;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ident {
    pub text: String,
    pub span: Span,
}

/// A name with an optional module qualifier: `auth::identities`, `User`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QName {
    pub module: Option<Ident>,
    pub name: Ident,
}

impl QName {
    pub fn span(&self) -> Span {
        match &self.module {
            Some(m) => m.span.to(self.name.span),
            None => self.name.span,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CtorSel {
    One(Ident),
    Many(Vec<Ident>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationRef {
    pub relation: QName,
    pub sel: Option<CtorSel>,
    pub span: Span,
}

/// `Type.Ctor`, with an optional module qualifier on the type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CtorPath {
    pub ty: QName,
    pub ctor: Ident,
}

/// A version written `12 (c41e...)`. `hash` is `None` where an example elides it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub number: u64,
    pub hash: Option<String>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleName {
    pub family: String,
    pub version: String,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeKind {
    Named {
        name: QName,
        sel: Option<CtorSel>,
        args: Vec<TypeArg>,
    },
    Var(Ident),
    /// `Key r`, with the span of the word `Key`.
    Key(Span, RelationRef),
    Unit,
    Record(Vec<FieldType>),
    Sum(Vec<AnonCtor>),
    Fn(Box<Type>, Box<Type>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeArg {
    Type(Type),
    Int(u64, Span),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnonCtor {
    pub name: Ident,
    pub record: Option<Vec<FieldType>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldType {
    pub name: Ident,
    pub ty: Type,
    pub on_delete: Option<FieldAction>,
}

/// A field's delete action, with the span of its text after `on delete`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldAction {
    pub action: RefAction,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefAction {
    Restrict,
    Cascade,
    Clear,
    Retag(Ident, Option<RecordExpr>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    Key,
    Try,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Union,
    Or,
    And,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Add,
    Sub,
    Concat,
    Mul,
    Div,
    Rem,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lit {
    Int(String),
    Decimal(String),
    Float(String),
    Text(String),
    Duration { amount: u64, unit: String },
    Bool(bool),
}

/// Returns the microseconds one `unit` of a duration literal stands for, `None` for a word that
/// is no duration unit.
pub fn duration_unit(unit: &str) -> Option<i64> {
    match unit {
        "us" => Some(1),
        "ms" => Some(1_000),
        "s" => Some(1_000_000),
        "min" => Some(60_000_000),
        "h" => Some(3_600_000_000),
        "d" => Some(86_400_000_000),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExprKind {
    Lit(Lit),
    Name(QName),
    Call {
        callee: QName,
        args: CallArgs,
        pins: Option<Version>,
    },
    TypeCall {
        ty: Type,
        func: Ident,
        args: CallArgs,
    },
    Ctor {
        path: QName,
        ctor: Option<Ident>,
        record: Option<RecordExpr>,
    },
    Record(RecordExpr),
    Set(Vec<Expr>),
    Unit,
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Field(Box<Expr>, Ident),
    Select(Box<Expr>, CtorSel),
    Backlink(Box<Expr>, RelationRef, Ident),
    KeyCall(Box<Expr>, KeyArgs),
    Comprehension(Box<Comprehension>),
    Match(Option<Box<Expr>>, Vec<Arm>),
    Fix(Vec<(Ident, Expr)>, Box<Expr>),
    Lambda(LambdaParams, Box<Expr>),
    Exists(Vec<Generator>, Option<Box<Expr>>),
    Top {
        count: Box<Expr>,
        keys: Vec<OrderKey>,
        body: Box<Comprehension>,
    },
    If(Box<Expr>, Box<Expr>, Box<Expr>),
    At {
        instant: bool,
        point: Box<Expr>,
        pins: Option<Version>,
        body: Box<Expr>,
    },
    Reject(Box<Expr>),
    Write(Box<Write>),
    /// `{..}` or `...` where an example leaves something out.
    Elided,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallArgs {
    List(Vec<Expr>),
    Aggregate {
        expr: Box<Expr>,
        generators: Vec<Generator>,
        filter: Option<Box<Expr>>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyArgs {
    Value(Box<Expr>),
    Named(Vec<(Ident, Expr)>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordExpr {
    Fields(Vec<(Ident, Expr)>, Span),
    With(Box<Expr>, Vec<(Ident, Expr)>, Span),
    Elided(Span),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LambdaParams {
    One(Ident),
    List(Vec<Param>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Param {
    pub name: Ident,
    pub ty: Type,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Generator {
    pub binder: Ident,
    pub source: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Clause {
    From(Vec<Generator>),
    Where(Expr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub value: Expr,
    pub by: Expr,
    pub into: Ident,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Comprehension {
    pub clauses: Vec<Clause>,
    pub group: Option<(Group, Vec<Clause>)>,
    pub select: Expr,
    pub order: Option<OrderClause>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderKey {
    pub expr: Expr,
    pub desc: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderClause {
    pub keys: Vec<OrderKey>,
    pub take: Option<Expr>,
    pub after: Option<Expr>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arm {
    pub patterns: Vec<Pattern>,
    pub guard: Option<Expr>,
    pub body: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub kind: PatternKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PatternKind {
    Wildcard,
    Bind(Ident),
    Ctor {
        path: QName,
        ctor: Option<Ident>,
        payload: Option<Payload>,
    },
    Lit {
        neg: bool,
        lit: Lit,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Payload {
    Wildcard,
    Bind(Ident),
    Record(Vec<FieldPat>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldPat {
    pub field: Ident,
    pub pattern: Option<Pattern>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Write {
    pub kind: WriteKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "every variant is common; goes if `Expr` itself is a pointer"
)]
pub enum WriteKind {
    Insert {
        target: RelationRef,
        from: bool,
        source: Expr,
        upsert: Option<Upsert>,
        rejects: Option<QName>,
    },
    Update {
        binder: Ident,
        source: Option<Expr>,
        filter: Option<Expr>,
        value: Expr,
    },
    Delete {
        binder: Ident,
        source: Option<Expr>,
        filter: Option<Expr>,
    },
    Fail(Expr),
    Skip,
    Seq(Vec<Write>),
    If(Expr, Box<Write>, Box<Write>),
    Match(Expr, Vec<WriteArm>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriteArm {
    pub patterns: Vec<Pattern>,
    pub guard: Option<Expr>,
    pub body: Write,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConflictTarget {
    Key,
    Fields(Vec<Expr>, Option<CtorScope>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upsert {
    pub on: ConflictTarget,
    pub otherwise: Box<Write>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TxnOption {
    Retry(u64),
    ReadOnly { deferrable: bool },
    Durability { quorum: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransactBlock {
    pub options: Vec<TxnOption>,
    pub items: Vec<BlockItem>,
    pub ret: Option<Expr>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockItem {
    Let(Ident, LetValue),
    Write(Box<Write>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LetValue {
    Expr(Box<Expr>),
    Write(Box<Write>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CtorScope {
    One(Ident),
    Many(Vec<Ident>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeDecl {
    pub name: Ident,
    pub params: Vec<Ident>,
    pub ordered: bool,
    pub header: Option<Vec<FieldType>>,
    pub body: TypeBody,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TypeBody {
    New(Type, Option<Box<Expr>>),
    Record(Vec<FieldType>),
    Ctors(Vec<CtorDef>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CtorDef {
    pub name: Ident,
    pub record: Option<Vec<FieldType>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RelationDecl {
    pub name: QName,
    pub ty: Type,
    pub clauses: Vec<RelationClause>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelationClause {
    Key(Vec<Ident>, Option<Ident>),
    Volatile,
    Unique(Vec<Expr>, Option<CtorScope>),
    Index {
        interval: bool,
        fields: Vec<Expr>,
        scope: Option<CtorScope>,
    },
    Default {
        ctor: Option<Ident>,
        field: Ident,
        value: DefaultValue,
    },
    Computed(Ident, Type, Expr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefaultValue {
    Generate(Ident),
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetDecl {
    pub name: Ident,
    pub relation: Ident,
    pub members: Vec<Ident>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeParam {
    pub name: Ident,
    pub bounds: Vec<Ident>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FnDecl {
    pub name: Ident,
    pub type_params: Vec<TypeParam>,
    pub params: Vec<Param>,
    pub result: Type,
    pub body: FnBody,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FnBody {
    Expr(Expr),
    Wasm {
        module: String,
        export: String,
    },
    Host {
        name: String,
        version: String,
        deterministic: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub limit: Option<Expr>,
    pub result: Type,
    pub body: Expr,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TxnDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub limit: Option<Expr>,
    pub result: Type,
    pub body: TransactBlock,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConstraintDecl {
    pub name: Ident,
    pub query: Expr,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckDecl {
    pub name: Ident,
    pub on: RelationRef,
    pub predicate: Expr,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoreDecl {
    pub view: QName,
    pub clauses: Vec<RelationClause>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AggregateDecl {
    pub name: Ident,
    pub params: Vec<Param>,
    pub result: Type,
    pub state: Vec<FieldType>,
    pub init: Expr,
    pub step: (Ident, Ident, Expr),
    pub merge: (Ident, Ident, Expr),
    pub finish: (Ident, Expr),
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decl {
    Type(TypeDecl),
    Relation(RelationDecl),
    Set(SetDecl),
    Fn(FnDecl),
    Constraint(ConstraintDecl),
    Check(CheckDecl),
    Store(StoreDecl),
    Aggregate(Box<AggregateDecl>),
}

impl Decl {
    pub fn span(&self) -> Span {
        match self {
            Decl::Type(d) => d.span,
            Decl::Relation(d) => d.span,
            Decl::Set(d) => d.span,
            Decl::Fn(d) => d.span,
            Decl::Constraint(d) => d.span,
            Decl::Check(d) => d.span,
            Decl::Store(d) => d.span,
            Decl::Aggregate(d) => d.span,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Operator {
    pub kind: OperatorKind,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldPath {
    pub ty: QName,
    pub ctor: Option<Ident>,
    pub field: Ident,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArmClause {
    pub target: QName,
    pub field: Option<Ident>,
    pub pattern: Pattern,
    pub body: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SplitClause {
    Unique(Vec<Expr>, CtorScope),
    Index {
        interval: bool,
        fields: Vec<Expr>,
        scope: CtorScope,
    },
    Arm(Box<ArmClause>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fill {
    Match(Vec<Arm>),
    Expr(Expr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CtorTarget {
    Relation(RelationRef),
    Type(CtorPath),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CtorPlace {
    Before(Ident),
    After(Ident),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OperatorKind {
    AddModule(Ident),
    DropModule(Ident),
    AddType(TypeDecl),
    DropType(QName),
    RenameType(QName, Ident),
    AddRelation(RelationDecl),
    DropRelation(QName),
    RenameRelation(QName, Ident),
    AddSet(SetDecl),
    DropSet(Ident, QName),
    RenameSet(Ident, QName, Ident),
    AddConstructor {
        path: CtorPath,
        record: Option<Vec<FieldType>>,
        place: Option<CtorPlace>,
        joins: Vec<RelationRef>,
        arms: Vec<ArmClause>,
    },
    RemoveConstructor(CtorPath),
    RenameConstructor(CtorPath, Ident),
    SplitConstructor {
        path: CtorPath,
        into: Vec<CtorDef>,
        arms: Vec<Arm>,
        clauses: Vec<SplitClause>,
    },
    MergeConstructor {
        ty: QName,
        sel: CtorSel,
        into: CtorDef,
        arms: Option<Vec<Arm>>,
        clauses: Vec<ArmClause>,
    },
    NestConstructor {
        ty: QName,
        sel: CtorSel,
        into: CtorDef,
        clauses: Vec<ArmClause>,
    },
    UnnestConstructor(CtorPath, Vec<ArmClause>),
    AddField {
        path: FieldPath,
        ty: Type,
        on_delete: Option<RefAction>,
        /// The field's name through its type and delete action.
        decl: Span,
        fill: Fill,
    },
    /// `alter field T.C.f on delete a`, with the span of `a`.
    AlterField {
        path: FieldPath,
        action: RefAction,
        span: Span,
    },
    DropField(FieldPath),
    RenameField(FieldPath, Ident),
    MoveField {
        path: FieldPath,
        to: QName,
        sel: Option<CtorSel>,
        fill: Option<Fill>,
    },
    AddUnique(Vec<Expr>, RelationRef),
    DropUnique(Vec<Expr>, RelationRef),
    AddIndex {
        interval: bool,
        fields: Vec<Expr>,
        on: RelationRef,
    },
    DropIndex {
        interval: bool,
        fields: Vec<Expr>,
        on: RelationRef,
    },
    Rekey {
        relation: QName,
        key: Vec<Ident>,
        carry: Vec<FieldPath>,
    },
    AddDefault {
        relation: QName,
        ctor: Option<Ident>,
        field: Ident,
        value: DefaultValue,
    },
    DropDefault {
        relation: QName,
        ctor: Option<Ident>,
        field: Ident,
    },
    DefineFn(FnDecl),
    RedefineFn(FnDecl),
    DropFn(QName),
    AddConstraint(ConstraintDecl),
    DropConstraint(QName),
    AddCheck(CheckDecl),
    DropCheck(QName),
    Store(StoreDecl),
    Unstore(QName),
    AddComputed {
        relation: QName,
        field: Ident,
        ty: Type,
        expr: Expr,
    },
    RedefineComputed {
        relation: QName,
        field: Ident,
        ty: Type,
        expr: Expr,
    },
    DropComputed(QName, Ident),
    DefineAggregate(Box<AggregateDecl>),
    DropAggregate(QName),
    Enable(CtorTarget),
    Disable(CtorTarget),
    Deprecate(CtorTarget),
    Revert {
        migration: String,
        fills: Vec<(FieldPath, Fill)>,
    },
    Script(Box<Write>),
    Install {
        name: Ident,
        version: String,
        from: String,
    },
    Uninstall(Ident),
    AdoptTzdb {
        release: String,
        from: String,
    },
    Grant(Grant),
    Revoke(Revoke),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallTarget {
    Bundle(BundleName),
    Family(String, Span),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Privilege {
    Read(RelationRef, Option<Vec<Ident>>),
    Insert(RelationRef),
    Update(RelationRef, Option<Vec<Ident>>),
    Delete(RelationRef),
    Retag(RelationRef, RelationRef),
    Call(CallTarget, Option<Ident>),
    CallFn(QName),
    Capability(Ident),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    pub privileges: Vec<Privilege>,
    pub to: Ident,
    pub grant_option: bool,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revoke {
    pub privileges: Vec<Privilege>,
    pub from: Ident,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HeaderClause {
    Lease(Expr),
    Priority(Ident),
    Budget(RecordExpr),
    HideNewConstructors,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleHeader {
    pub name: BundleName,
    pub pins: Version,
    pub catalog: Option<Version>,
    pub edition: String,
    pub clauses: Vec<HeaderClause>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Import {
    Module(Ident),
    Fn(QName),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedDecl {
    pub name: Ident,
    pub of: RelationRef,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BundleItem {
    Fn(FnDecl),
    Query(QueryDecl),
    Txn(TxnDecl),
    Feed(FeedDecl),
    Aggregate(Box<AggregateDecl>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaFile {
    pub module: Ident,
    pub decls: Vec<Decl>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MigrationFile {
    pub module: Ident,
    pub operators: Vec<Operator>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BundleFile {
    pub header: BundleHeader,
    pub imports: Vec<Import>,
    pub items: Vec<BundleItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoleStmt {
    Create(Ident),
    AlterPassword(Ident, Box<Expr>),
    Drop(Ident),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeerStmt {
    Map {
        pattern: String,
        listener: Ident,
        role: Ident,
    },
    Unmap {
        ordinal: u64,
        listener: Ident,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TempDecl {
    pub name: Ident,
    pub ty: Type,
    pub key: Option<(Vec<Ident>, Option<Ident>)>,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExplainTarget {
    Registered {
        bundle: BundleName,
        query: Ident,
        args: CallArgs,
    },
    Expr(Expr),
    Write(Box<Write>),
    Transact(TransactBlock),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RestorePoint {
    BeforeVersion(u64),
    AtInstant(Expr),
    AtPosition(Expr),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Explain {
        analyze: bool,
        target: ExplainTarget,
    },
    PinPlan(Expr),
    UnpinPlan(Expr),
    CancelCall(Expr),
    TerminateSession(Expr),
    Vacuum(QName),
    Verify(Option<QName>),
    Reindex {
        relation: QName,
        index: Option<(Vec<Expr>, CtorScope)>,
        online: Option<bool>,
    },
    Compact,
    FinalizeUpgrade,
    RetireBundle(BundleName),
    SupersedeBundle(BundleName, BundleName),
    SetSetting(Ident, Expr),
    Export {
        source: Expr,
        path: String,
        sums_text: bool,
    },
    RegisterBundle(String),
    ApplyMigrations(Vec<String>),
    PrintSchema,
    ArchiveAudit {
        through: Expr,
        path: String,
    },
    RotateKek(String),
    RotateDek,
    DestroyDek(u64),
    Dump {
        path: String,
        encrypt: Option<Expr>,
    },
    Restore {
        from: String,
        to: String,
        point: Option<RestorePoint>,
    },
    Backup {
        path: String,
        incremental: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdhocItem {
    Expr(Expr),
    Write(Box<Write>),
    Transact(TransactBlock),
    Command(Command, Span),
    Grant(Grant),
    Revoke(Revoke),
    Role(RoleStmt, Span),
    Peer(PeerStmt, Span),
    Temp(TempDecl),
}

/// One item of a documentation example, which may mix every kind of file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExampleItem {
    Module(Ident),
    Decl(Decl),
    Operator(Operator),
    BundleHeader(BundleHeader),
    Import(Import),
    BundleItem(BundleItem),
    Adhoc(AdhocItem),
}
