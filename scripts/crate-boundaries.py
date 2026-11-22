# SPDX-License-Identifier: AGPL-3.0-or-later
"""Reject Rust source includes outside the repository."""

import json
import os
from pathlib import Path
import re
import sys
import tempfile

# A token keeps its source offset so diagnostics retain the original line.
string_token = re.compile(r'(?:[bc]?r(?P<hash>\#*)"(?P<raw>.*?)"(?P=hash)|[bc]?"(?P<text>(?:\\.|[^"\\])*)")', re.S)
character = re.compile(r"(?:b)?'(?:\\(?:u\{[0-9a-fA-F_]+\}|x[0-9a-fA-F]{2}|.)|[^'\\\n])'", re.S)
identifier = re.compile(r'[A-Za-z_][A-Za-z_0-9]*')
escape = re.compile(r'\\(?:u\{([0-9a-fA-F_]+)\}|x([0-9a-fA-F]{2})|\r?\n\s*|(.))', re.S)


def string_value(match):
    if match['raw'] is not None:
        return match['raw']
    def replace(part):
        if part[1] is not None:
            return chr(int(part[1].replace('_', ''), 16))
        if part[2] is not None:
            return chr(int(part[2], 16))
        return {'n': '\n', 'r': '\r', 't': '\t', '0': '\0'}.get(part[3], part[3] or '')
    return escape.sub(replace, match['text'])


def tokens(text):
    result = []
    offset = 0
    while offset < len(text):
        if text[offset].isspace():
            offset += 1
        elif text.startswith('//', offset):
            end = text.find('\n', offset)
            offset = len(text) if end < 0 else end
        elif text.startswith('/*', offset):
            depth = 1
            offset += 2
            while offset < len(text) and depth:
                if text.startswith('/*', offset):
                    depth += 1
                    offset += 2
                elif text.startswith('*/', offset):
                    depth -= 1
                    offset += 2
                else:
                    offset += 1
        else:
            string = string_token.match(text, offset)
            char = character.match(text, offset)
            word = identifier.match(text, offset)
            if string:
                result.append(('string', string_value(string), offset))
                offset = string.end()
            elif char:
                result.append(('char', char[0], offset))
                offset = char.end()
            elif word:
                result.append(('word', word[0], offset))
                offset = word.end()
            else:
                result.append(('punct', text[offset], offset))
                offset += 1
    return result


def group_end(items, start):
    closing = {'(': ')', '[': ']', '{': '}'}
    if start >= len(items) or items[start][0] != 'punct' or items[start][1] not in closing:
        return None
    stack = [closing[items[start][1]]]
    for index in range(start + 1, len(items)):
        kind, value, _ = items[index]
        if kind != 'punct':
            continue
        if value in closing:
            stack.append(closing[value])
        elif value in closing.values():
            if value != stack.pop():
                return None
            if not stack:
                return index
    return None


def include_value(items, directory):
    if items and items[-1][:2] == ('punct', ','):
        items = items[:-1]
    if len(items) == 1 and items[0][0] == 'string':
        return items[0][1]
    if len(items) < 4 or items[0][0] != 'word' or items[1][1] != '!':
        return None
    end = group_end(items, 2)
    if end != len(items) - 1:
        return None
    arguments = items[3:end]
    if items[0][1] == 'env':
        if arguments and arguments[-1][:2] == ('punct', ','):
            arguments = arguments[:-1]
        if len(arguments) == 1 and arguments[0][:2] == ('string', 'CARGO_MANIFEST_DIR'):
            return str(directory)
    elif items[0][1] == 'concat':
        parts = []
        start = index = 0
        while index < len(arguments):
            if arguments[index][0] == 'punct' and arguments[index][1] in ('(', '[', '{'):
                end = group_end(arguments, index)
                if end is None:
                    return None
                index = end
            elif arguments[index][:2] == ('punct', ','):
                value = include_value(arguments[start:index], directory)
                if value is None:
                    return None
                parts.append(value)
                start = index + 1
            index += 1
        if start < len(arguments):
            value = include_value(arguments[start:], directory)
            if value is None:
                return None
            parts.append(value)
        return ''.join(parts)
    return None


def source_includes(text, directory):
    items = tokens(text)
    for index, (kind, value, offset) in enumerate(items):
        if kind == 'word' and value in ('include', 'include_str', 'include_bytes'):
            if index + 2 >= len(items) or items[index + 1][1] != '!':
                continue
            end = group_end(items, index + 2)
            if end is not None:
                path = include_value(items[index + 3:end], directory)
                if path is not None:
                    yield offset, path
        elif kind == 'punct' and value == '#' and index + 5 < len(items):
            if [item[1] for item in items[index + 1:index + 4]] == ['[', 'path', '=']:
                end = group_end(items, index + 1)
                if end is not None:
                    path = include_value(items[index + 4:end], directory)
                    if path is not None:
                        yield offset, path


def check(packages, repository_root=None):
    owners = sorted([(Path(p['manifest_path']).parent.resolve(), p['name'],
                      p['metadata']['unemu'].get('layer') == 'emulation'
                      or p['metadata']['unemu'].get('side') == 'core')
                     for p in packages], key=lambda p: len(p[0].parts), reverse=True)
    repository_root = Path(repository_root or Path(os.path.commonpath([str(p[0]) for p in owners]))).resolve()
    def owner(path):
        return next((p for p in owners if path.is_relative_to(p[0])), None)
    errors = []
    for directory, name, core in owners:
        for source in sorted(directory.rglob('*.rs')):
            if owner(source)[1] != name:
                continue
            text = source.read_text()
            for offset, value in source_includes(text, directory):
                target = (source.parent / value).resolve()
                other = owner(target)
                if not target.is_relative_to(repository_root):
                    line = text.count('\n', 0, offset) + 1
                    errors.append(f'outside repository source include: {name} in {source.relative_to(directory)}:{line}')
                elif other is not None and other[2] != core:
                    line = text.count('\n', 0, offset) + 1
                    errors.append(f'forbidden source include: {name} → {other[1]} in {source.relative_to(directory)}:{line}')
    for error in errors:
        print(error)
    return len(errors)

if sys.argv[1] == '--self-test':
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        packages = []
        for name, core in [('front', False), ('core', True), ('peer', False)]:
            directory = root / name
            directory.mkdir()
            packages.append(dict(name=name, manifest_path=str(directory / 'Cargo.toml'),
                                 metadata={'unemu': {'layer': 'pc', 'side': 'core' if core else 'frontend'}}))
            (directory / 'lib.rs').write_text('')
        for expression in ['#[path = "../core/lib.rs"] mod shared;',
                           'include!("../core/lib.rs");',
                           'include_str!(\n"../core/lib.rs");',
                           'include_bytes!(r"../core/lib.rs");',
                           '#[path = r#"../core/lib.rs"#] mod shared;',
                           'include! { "../core/lib.rs" }',
                           'include_bytes!["../core/lib.rs"];',
                           'include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../core/lib.rs"));',
                           'include_str!(concat!("../", "core/lib.rs"));',
                           'include_str!(concat ! (env!("CARGO_MANIFEST_DIR"), "/../core/lib.rs"));',
                           'include_bytes! { concat /* spacing */ ! ["../", concat!("core/", r#"lib.rs"#),] }',
                           r'include!("../core/\x6cib.rs");',
                           '/* outer /* inner */ done */\ninclude!("../core/lib.rs");']:
            (root / 'front/lib.rs').write_text(expression)
            assert check(packages) == 1, expression
        for expression in [
                'include!("../peer/lib.rs");\n// include!("../core/lib.rs");',
                'include!(concat ! ("../", "peer/lib.rs"));',
                'let example = r#"include_str!("../core/lib.rs");"#;',
                'let example = r###"#[path = "../core/lib.rs"] mod hidden;"###;',
                'let example = "include!(\\\"../core/lib.rs\\\");";',
                '/* outer /* inner */ include!("../core/lib.rs"); */',
                '/* outer /* inner /* deeper */ */ include!("../core/lib.rs"); */',
                'let example = br#"include_bytes!("../core/lib.rs");"#;',
                'let punctuation = (\')\', \'"\');',
        ]:
            (root / 'front/lib.rs').write_text(expression)
            assert check(packages) == 0, expression
        (root / 'front/lib.rs').write_text('')
        (root / 'core/lib.rs').write_text('include!("../front/lib.rs");')
        assert check(packages) == 1
        (root / 'core/lib.rs').write_text('')
        nested = root / 'front/src'
        nested.mkdir()
        (root / 'front/lib.rs').write_text('')
        for expression in [
                'include_str!(concat!("../../", "core/lib.rs"));',
                'include_str!(concat ! (env!("CARGO_MANIFEST_DIR"), "/../core/lib.rs"));',
                'include_str!(concat!("../../", "core/", ")", ".rs"));',
        ]:
            (nested / 'lib.rs').write_text(expression)
            assert check(packages) == 1, expression
        (nested / 'lib.rs').write_text('')
        (root / 'core/lib.rs').write_text('include_str!("../../outside.txt");')
        assert check(packages) == 1, 'includes outside the repository must fail'
    print('self-test source includes: cross-side plants rejected; same-side includes and comments accepted')
else:
    metadata = json.loads(Path(sys.argv[1]).read_text())
    errors = check(metadata['packages'], metadata['workspace_root'])
    print(f'crate boundaries: {errors} source-include error(s)')
    sys.exit(bool(errors))
