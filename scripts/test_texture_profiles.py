"""Compare local Live2D texture profiles; requires the user's local model/Chrome.

No API requests. Original model bytes are hashed before and after the probes.
"""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

ROOT=Path(__file__).resolve().parents[1]
ASSETS=Path(os.environ.get('ASTER_PET_DIR',str(Path.home()/'desktop-pet/assets')))

def main():
    model=ASSETS/'弄玉运行档_无水印'
    definition=json.loads((model/'弄玉.model3.json').read_text())
    paths=[model/'弄玉.model3.json']+[model/x for x in definition['FileReferences']['Textures']]
    paths += [model/definition['FileReferences'][name] for name in ('Moc','Physics','DisplayInfo') if name in definition['FileReferences']]
    def hashes():return {str(p.relative_to(model)):hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}
    before=hashes();results=[]
    for limit in [4096,2048,1024]:
        output=ROOT/'.aster/qa'/f'textures-{limit}';start=time.monotonic()
        run=subprocess.run([str(ROOT/'aster'),'--state-dir',str(output/'state'),'--texture-size',str(limit),'--live2d-probe',str(output)],cwd=ROOT,capture_output=True,text=True,timeout=85)
        assert run.returncode==0,run.stderr
        data=json.loads((output/'renderer.json').read_text());info=data['info']
        assert data['animation_changes'] and data['frames']>=9
        assert info['texture_limit']==limit and not info['texture_warnings']
        assert len(info['texture_sizes'])==len(definition['FileReferences']['Textures'])
        assert all(max(t['rendered'])<=limit for t in info['texture_sizes'])
        original=sum(t['original'][0]*t['original'][1] for t in info['texture_sizes'])
        rendered=sum(t['rendered'][0]*t['rendered'][1] for t in info['texture_sizes'])
        evidence={'limit':limit,'seconds':round(time.monotonic()-start,3),'textures':len(info['texture_sizes']),'original_pixels':original,'rendered_pixels':rendered,'pixel_reduction_percent':round(100*(1-rendered/original),2),'frames':data['frames'],'attempts':info['attempt']}
        results.append(evidence);print(json.dumps(evidence),flush=True)
    assert hashes()==before,'Original assets changed'
    evidence={'profiles':results,'original_assets_unchanged':True,'api_calls':0}
    (ROOT/'.aster/qa/texture-profiles.json').write_text(json.dumps(evidence,indent=2))

if __name__=='__main__':main()
