import csv,sys,glob,os
sys.path.insert(0,'/home/user/study/scripts')
from rows import rows
D='/home/user/s106-data/'
sch,vars_=sys.argv[1],sys.argv[2].split(',')
srcs=sys.argv[3].split(',') if len(sys.argv)>3 else None
for f in sorted(glob.glob(D+'proposals/2026-*.csv')+glob.glob(D+'labels/*.csv')):
    for i,r in rows(f):
        if r.get('scheme')==sch and r.get('variable_id') in vars_ and (not srcs or r['source'] in srcs):
            print(f"{os.path.relpath(f,D)}:{i} {r['labeller'][-16:]} {r['variable_id']}/{r['source']} [{r['status']}] basis={r['basis']} pages={r['pages']}\n    V: {r['value'][:230]}\n    N: {' '.join(r['notes'].split())[:380]}")
