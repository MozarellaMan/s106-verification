import csv,sys
sys.path.insert(0,'/home/user/study/scripts')
from rows import rows
sys.path.insert(0,'/home/user/study/scripts')
from decimal import Decimal
D='/home/user/s106-data/'
def rd(f):
    return [dict(r,_f=f,_l=i) for i,r in rows(D+f)]
def kv(v):
    o={}
    for p in v.split(';'):
        if '=' in p:
            k,x=p.split('=',1);o[k.strip()]=x.strip()
    return o
def num(x):
    try:return float(x.replace(',',''))
    except:return None
def tot(v):
    f=kv(v);t=num(f.get('total','')) 
    m=num(f.get('market','')) if 'market' in f else None
    ten=[num(x) for k,x in f.items() if k not in('total','market','basis') and num(x) is not None]
    a=(t-m) if (t is not None and m is not None) else (sum(ten) if ten else None)
    return t,a
dh={r['scheme']:r for r in rd('proposals/2026-10-14-datahub-mapping-batch1.csv') if r['variable_id']=='V1'}
src=[('proposals/2026-10-14-claude-opus-batch1-documents.csv','deed'),('proposals/2026-10-14-agents-batch1-documents.csv','deed'),('proposals/2026-10-14-claude-opus-followup-documents.csv','variation'),('proposals/2026-10-14-claude-opus-followup-documents.csv','committee_report'),('proposals/2026-10-14-claude-opus-batch1-documents.csv','committee_report'),('proposals/2026-10-14-claude-opus-batch1-documents.csv','gla'),('labels/2026-10-14-ab-audit-priority-batch1.csv',None)]
allr=[]
for f,s in src:
    for r in rd(f):
        if r['variable_id']=='V1' and (s is None or r['source']==s): allr.append(r)
for sch,d in sorted(dh.items()):
    dt,da=tot(d['value'])
    print('\n==',sch,'DATAHUB',d['value'],'-> total',dt,'affordable',da,'(%s:%d)'%(d['_f'],d['_l']))
    for r in allr:
        if r['scheme']!=sch: continue
        if r['status']!='found' and r['status']!='ambiguous': 
            print('   %-9s %-12s status=%s  (%s:%d)'%(r['labeller'][-14:],r['source'],r['status'],r['_f'],r['_l']));continue
        t,a=tot(r['value'])
        flag='' if (t==dt and a==da) else '  <-- DIFFERS'
        print('   %-14s %-16s %-9s total=%s aff=%s basis=%s | %s (%s:%d)%s'%(r['labeller'][-14:],r['source'],r['status'],t,a,r['basis'],r['value'][:110],r['_f'],r['_l'],flag))
