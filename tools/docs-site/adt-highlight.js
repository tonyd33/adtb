function adtLanguage(hljs) {
    return {
        name: 'adtb',
        keywords: {
            keyword: 'and bundle by check constraint delete else exists fail false fix fn for from group if import in insert into key let match module not or order ordered query relation return schema select set skip temp then top transact true try txn type union update where with'
        },
        contains: [
            hljs.COMMENT('--', '$'),
            hljs.QUOTE_STRING_MODE,
            {className: 'number', begin: /\b[0-9]+(?:\.[0-9]+)?(?:[eE][+-]?[0-9]+|us\b|ms\b|min\b|s\b|h\b|d\b)?/},
            {className: 'type', begin: /\b[A-Z][A-Za-z0-9_]*/}
        ]
    };
}
hljs.registerLanguage('adt', adtLanguage);
if (typeof document !== 'undefined') {
    document.querySelectorAll('code.language-adt').forEach(block => hljs.highlightBlock(block));
}
