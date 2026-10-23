#!/usr/bin/env python3
"""Validate version tags and package CPU binaries with third-party license texts."""
import argparse
import hashlib
import json
import pathlib
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile

ROOT = pathlib.Path(__file__).resolve().parents[1]
TARGETS = {
    'x86_64-unknown-linux-gnu', 'aarch64-unknown-linux-gnu',
    'x86_64-apple-darwin', 'aarch64-apple-darwin', 'x86_64-pc-windows-msvc',
}

SUPPLEMENTAL = {
    ('block2','0.6.2'):'b4167b582b2f75f9a1be75495c41b765344fd03c',
    ('dispatch2','0.3.1'):'8852b424193ca41602281b3d7540d7c8ed51e49a',
    ('objc2','0.6.5'):'d7d2fa23ceaa5e6096c923b081040e5d81b3b9df',
    ('objc2-encode','4.1.0'):'8d214f5477365ffcbcbb7de058c86ed9a518efb7',
}


def run(*args):
    return subprocess.check_output(args, cwd=ROOT, text=True, encoding="utf-8").strip()


def validate_tag(tag):
    match = re.fullmatch(r'v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?', tag)
    if not match or (match[4] and any(x.isdigit() and len(x)>1 and x[0]=='0' for x in match[4].split('.'))):
        raise ValueError('tag must be v followed by a SemVer version, e.g. v0.1.0')
    version = tomllib.loads((ROOT/'Cargo.toml').read_text())['package']['version']
    if tag[1:] != version:
        raise ValueError(f'tag {tag} does not match Cargo.toml version {version}')
    commit = run('git','rev-parse','HEAD')
    tagged = run('git','rev-parse','--verify',f'refs/tags/{tag}^{{commit}}')
    if commit != tagged:
        raise ValueError('checkout must be the exact commit referenced by the version tag')
    return version, commit


def collect_licenses(target, destination):
    metadata = json.loads(run('cargo','metadata','--locked','--format-version','1','--filter-platform',target))
    packages = {p['id']:p for p in metadata['packages']}
    nodes = {n['id']:n for n in metadata['resolve']['nodes']}
    root = metadata['resolve']['root']
    pending = [root]
    reachable = set()
    while pending:
        ident = pending.pop()
        if ident in reachable:
            continue
        reachable.add(ident)
        pending.extend(dep['pkg'] for dep in nodes[ident]['deps'])
    inventory=[]
    for ident in sorted(reachable):
        if ident == root:
            continue
        package = packages[ident]
        source = pathlib.Path(package['manifest_path']).parent
        directory = destination/f"{package['name']}-{package['version']}"
        candidates=set()
        if package.get('license_file'):
            candidates.add(source/package['license_file'])
        for path in source.iterdir():
            upper=path.name.upper()
            if path.is_file() and upper.startswith(('LICENSE','LICENCE','COPYING','NOTICE','COPYRIGHT','AUTHORS')):
                candidates.add(path)
            if path.is_dir() and upper in ('LICENSES','LICENCES'):
                candidates.update(p for p in path.rglob('*') if p.is_file())
        supplement = None
        if not any(p.name.upper().startswith(('LICENSE','LICENCE','COPYING')) for p in candidates):
            expected = SUPPLEMENTAL.get((package['name'],package['version']))
            vcs = source/'.cargo_vcs_info.json'
            if expected and vcs.is_file() and json.loads(vcs.read_text())['git']['sha1']==expected:
                supplement=ROOT/'LICENSE'
            else:
                raise ValueError(f"no license text found for {package['name']} {package['version']}")
        directory.mkdir(parents=True)
        if supplement:
            # The root LICENSE retains the exact upstream MIT and scope notices.
            shutil.copyfile(supplement,directory/'LICENSE')
        for path in sorted(candidates):
            relative=path.relative_to(source)
            to=directory/relative
            to.parent.mkdir(parents=True,exist_ok=True)
            shutil.copyfile(path,to)
        inventory.append({'name':package['name'],'version':package['version'],
                          'license':package['license'],'source':package['source'],
                          'license_files':[str(p.relative_to(source)) for p in sorted(candidates)],
                          'supplemental_notices': 'LICENSE (objc2 family notices)' if supplement else None})
    destination.mkdir(parents=True,exist_ok=True)
    (destination/'inventory.json').write_text(json.dumps(inventory,indent=2)+'\n')


def package(target, version, commit, output):
    if target not in TARGETS:
        raise ValueError(f'unsupported target: {target}')
    exe='charart.exe' if 'windows' in target else 'charart'
    binary=ROOT/'target'/target/'release'/exe
    if not binary.is_file():
        raise ValueError(f'build the target first: {target}')
    actual=run(str(binary),'--version')
    if actual != f'charart {version}':
        raise ValueError(f'binary version mismatch: {actual}')
    output.mkdir(parents=True,exist_ok=True)
    name=f'charart-v{version}-{target}'
    with tempfile.TemporaryDirectory(prefix='charart-release-') as temporary:
        folder=pathlib.Path(temporary)/name
        folder.mkdir()
        shutil.copy2(binary,folder/exe)
        for filename in ['README.md','LICENSE']:
            shutil.copy2(ROOT/filename,folder/filename)
        (folder/'BUILD.json').write_text(json.dumps({'version':version,'commit':commit,'target':target,
           'backend':'cpu','cuda_enabled':False,'rustc':run('rustc','--version'),
           'binary_sha256':hashlib.sha256(binary.read_bytes()).hexdigest()},indent=2)+'\n')
        collect_licenses(target,folder/'third-party-licenses')
        if 'windows' in target:
            archive=output/f'{name}.zip'
            with zipfile.ZipFile(archive,'w',compression=zipfile.ZIP_DEFLATED) as zipped:
                for path in sorted(folder.rglob('*')):
                    if path.is_file():
                        zipped.write(path,path.relative_to(folder.parent))
        else:
            archive=output/f'{name}.tar.gz'
            with tarfile.open(archive,'w:gz') as packed:
                packed.add(folder,arcname=name)
    digest=hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_name(archive.name+'.sha256').write_text(f'{digest}  {archive.name}\n')
    print(archive)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--tag',required=True)
    parser.add_argument('--target',choices=sorted(TARGETS))
    parser.add_argument('--output',type=pathlib.Path,default=ROOT/'dist')
    args=parser.parse_args()
    version,commit=validate_tag(args.tag)
    if args.target:
        package(args.target,version,commit,args.output.resolve())
    else:
        print(f'validated {args.tag} at {commit}')


if __name__=='__main__':
    main()
