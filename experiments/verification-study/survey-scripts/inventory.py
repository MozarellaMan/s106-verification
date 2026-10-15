import csv,glob,collections,os,sys
D='/home/user/s106-data/'
rows=[]
for f in sorted([f for f in glob.glob(D+'proposals/*.csv') if 'superseded' not in f]+glob.glob(D+'labels/*.csv')):
    with open(f,newline='') as fh:
        r=csv.DictReader(fh)
        if 'labeller' not in (r.fieldnames or []): print('SKIP (no labeller col)',os.path.basename(f));continue
        for i,row in enumerate(r,2):
            row['_f']=os.path.relpath(f,D);row['_l']=i;rows.append(row)
print(len(rows))
c=collections.Counter((r['_f'],r['labeller']) for r in rows)
for k,v in sorted(c.items()):print(v,k)
print('--- per file: schemes, sources')
for f in sorted({r['_f'] for r in rows}):
    rs=[r for r in rows if r['_f']==f]
    print(f,'schemes',len({r['scheme'] for r in rs}),'sources',dict(collections.Counter(r['source'] for r in rs)),'vars',dict(sorted(collections.Counter(r['variable_id'] for r in rs).items())))
# multi-reader cells
cells=collections.defaultdict(set)
for r in rows: cells[(r['scheme'],r['variable_id'],r['source'])].add(r['labeller'])
sch=collections.defaultdict(lambda:collections.defaultdict(set))
for (s,v,src),L in cells.items():
    for l in L: sch[s][l].add((v,src))
print('--- schemes with readings from 2+ readers (distinct labellers)')
for s in sorted(sch):
    if len(sch[s])>=2: print(s,{l[8:]if l.startswith('machine:') else l:len(x) for l,x in sch[s].items()})
print('--- readings per (scheme,variable,source) distribution')
print(collections.Counter(len(L) for L in cells.values()))
print('cells',len(cells))
print('--- per (scheme, source) reading counts')
ss=collections.Counter((r['scheme'],r['source']) for r in rows)
for k,v in sorted(ss.items()):print(k,v)
