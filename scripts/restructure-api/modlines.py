import os, re, sys
src = sys.argv[1]
NOAPPLY = {'library', 'mcp', 'app', 'clipboard', 'terminal'}
for f in sorted(os.listdir(src)):
    d = f'{src}/{f}'
    if not os.path.isdir(d):
        continue
    mod = open(f'{d}/mod.rs').read()
    lines = []
    subs = [s for s in ('apply', 'attachment', 'command', 'event') if os.path.exists(f'{d}/{s}.rs')]
    for sub in subs:
        lines.append(f'mod {sub};')
    for sub in subs:
        if sub == 'apply':
            lines.append('pub(crate) use apply::apply;')
            continue
        names = re.findall(r'^pub (?:struct|enum|fn|const|type)\s+(\w+)', open(f'{d}/{sub}.rs').read(), re.M)
        lines.append(f'pub use {sub}::{{{", ".join(names)}}};' if len(names) > 1 else f'pub use {sub}::{names[0]};')
    ms = mod.split('\n')
    i = 0
    while i < len(ms) and ms[i].startswith('//!'):
        i += 1
    mod = '\n'.join(ms[:i]) + '\n\n' + '\n'.join(lines) + '\n' + '\n'.join(ms[i:])
    open(f'{d}/mod.rs', 'w').write(mod)
