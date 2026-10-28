for (const item of corpus) {
    const highlighted = hljs.highlight('adt', item.source, true).value;
    const entities = {'&amp;': '&', '&lt;': '<', '&gt;': '>', '&quot;': '"', '&#x27;': "'"};
    const plain = highlighted.replace(/<[^>]*>/g, '').replace(/&(amp|lt|gt|quot);|&#x27;/g, value => entities[value]);
    if (plain !== item.source) throw new Error(item.name + ': highlighting changed source text');
    for (const kind of item.absent || []) {
        if (highlighted.includes('class="hljs-' + kind + '"')) {
            throw new Error(item.name + ': unexpected ' + kind + ' token');
        }
    }
    for (const [kind, text] of item.tokens) {
        const encoded = text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;').replace(/'/g, '&#x27;');
        if (!highlighted.includes('<span class="hljs-' + kind + '">' + encoded + '</span>')) {
            throw new Error(item.name + ': missing ' + kind + ' token ' + JSON.stringify(text));
        }
    }
}
print('docs: highlighted ' + corpus.length + ' syntax inputs with preserved text and expected token spans');
