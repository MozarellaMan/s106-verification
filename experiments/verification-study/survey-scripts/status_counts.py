import csv,glob,collections,sys,os
sys.path.insert(0,'/home/user/study/scripts')
from rows import rows
D='/home/user/s106-data/'
fs=[f for f in sorted(glob.glob(D+'proposals/2026-*.csv')+glob.glob(D+'labels/*.csv'))]
c=collections.defaultdict(collections.Counter); tot=collections.Counter()
cf=collections.defaultdict(collections.Counter)
for f in fs:
    for i,r in rows(f):
        if 'labeller' not in r or not r.get('status'): continue
        k=(os.path.relpath(f,D),r['labeller']); c[k][r['status']]+=1; tot[k]+=1
sts=['found','ambiguous','inconsistent','not_stated','not_applicable','source_missing','no_source','unreadable']
print('file | labeller | n | '+' | '.join(sts))
for k in sorted(c): print(k[0],'|',k[1],'|',tot[k],'|',' | '.join(str(c[k][s]) for s in sts))
print('\nBY READER (all files pooled)')
rd=collections.defaultdict(collections.Counter)
for (f,l),cc in c.items(): rd[l].update(cc)
for l,cc in rd.items(): n=sum(cc.values());print(l,n,dict(cc),'abstain(not_stated+source_missing+no_source+unreadable)=%d (%.0f%%)'%(sum(cc[s] for s in['not_stated','source_missing','no_source','unreadable']),100*sum(cc[s] for s in['not_stated','source_missing','no_source','unreadable'])/n))
