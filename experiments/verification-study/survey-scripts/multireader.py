import csv,glob,collections,sys,os
sys.path.insert(0,'/home/user/study/scripts')
from rows import rows
D='/home/user/s106-data/'
cells=collections.defaultdict(set)
for f in sorted(glob.glob(D+'proposals/2026-*.csv')+glob.glob(D+'labels/*.csv')):
    for i,r in rows(f):
        if 'labeller' not in r or not r.get('status') or r['labeller'].endswith('sources-status'): continue
        cells[(r['scheme'],r['variable_id'],r['source'])].add(r['labeller'].replace('machine:proposal/',''))
print('cells with >=1 reading (excl. retrieval record):',len(cells))
print('readings-per-cell distribution (distinct labellers):',sorted(collections.Counter(len(v) for v in cells.values()).items()))
by=collections.defaultdict(lambda:collections.Counter())
for (s,v,src),L in cells.items():
    if len(L)>=2: by[s][tuple(sorted(L))]+=1
for s in sorted(by): print(s,dict(by[s]))
# cells with >=2 readers where readers are not just fable/opus: independent family
fam=lambda l:'person' if l=='AB' else ('datahub' if 'datahub' in l else l.split('-')[0])
print('cells with >=2 distinct FAMILIES:',sum(1 for L in cells.values() if len({fam(l) for l in L})>=2))
print('cells with claude+person:',sum(1 for L in cells.values() if {'claude','person'}<= {fam(l) for l in L}))
print('cells with claude+datahub:',sum(1 for L in cells.values() if {'claude','datahub'}<= {fam(l) for l in L}))
print('cells with person + datahub:',sum(1 for L in cells.values() if {'person','datahub'}<= {fam(l) for l in L}))
