import csv,collections,sys
sys.path.insert(0,'/home/user/study/scripts')
from rows import rows
from decimal import Decimal,InvalidOperation
D='/home/user/s106-data/proposals/'
def load(f):
    out={}
    for i,r in rows(D+f):
        r['_l']=i; out[(r['scheme'],r['variable_id'])]=r
    return out
def normal(s):
    w=' '.join(s.split()).lower()
    n=w.lstrip('£').rstrip('%').replace(',','')
    try: return str(Decimal(n).normalize())
    except InvalidOperation: return w
def fields(r):
    out={}
    if r['status']!='found': return out
    v=r['value']
    if '=' in v:
        for part in v.split(';'):
            if '=' in part:
                k,val=part.split('=',1); k=normal(k)
                if k=='other' and ':' in val:
                    lab,n=val.rsplit(':',1); out['other:'+normal(lab)]=normal(n)
                else: out[k]=normal(val)
    elif v.strip(): out['value']=normal(v)
    b=r['basis'].strip()
    if b and b!='not_stated': out['basis']=normal(b)
    return out
def compared(r):
    f=fields(r);var=r['variable_id']
    keep=lambda ks:{k:v for k,v in f.items() if k in ks}
    if var=='V1': f.pop('basis',None); return f
    if var=='V2': return keep(['share','basis','split'])
    if var=='V3':
        f=keep(['target','first_offer'])
        if 'target' in f: f['target']=(f['target'].split() or [''])[0]
        return f
    if var=='V4': return keep(['route','reviews'])
    if var=='V6': return keep(['cascade','pil','pil_amount'])
    if var=='V7' and r['status']=='found':
        import re
        ms=sorted({w[:7] for w in re.split(r'[^0-9-]',r['value']) if len(w)>=7 and w[4]=='-'})
        return {'months':' '.join(ms)}
    if var=='V8': return keep(['completed','completed_homes'])
    return {}
R1=load('2026-10-14-claude-opus-batch2-r1.csv');R2=load('2026-10-14-claude-opus-batch2-r2.csv')
FR1='proposals/2026-10-14-claude-opus-batch2-r1.csv';FR2='proposals/2026-10-14-claude-opus-batch2-r2.csv'
keys=sorted(R1,key=lambda k:(k[1],k[0]))
assert set(R1)==set(R2)
tot=collections.Counter();per=collections.defaultdict(lambda:[0,0]);fld=collections.defaultdict(lambda:[0,0]);vv=collections.defaultdict(lambda:[0,0,0])
dis=[]
for k in keys:
    a,b=R1[k],R2[k];v=k[1]
    sa=a['status']==b['status']; ca,cb=compared(a),compared(b)
    full=sa and ca==cb
    vv[v][1]+=1; vv[v][0]+=full; vv[v][2]+=sa
    if a['status']=='found' and b['status']=='found':
        for f in sorted(set(ca)|set(cb)):
            fld[(v,f)][1]+=1; fld[(v,f)][0]+= ca.get(f)==cb.get(f)
    if not full: dis.append((k,a,b,ca,cb,sa))
print('PER VARIABLE: cells, status-agree, full agree (status+compared fields)')
T=[0,0,0]
for v in sorted(vv): 
    x=vv[v];print(v,'cells',x[1],'status-agree',x[2],'full-agree',x[0],'= %.0f%%'%(100*x[0]/x[1]));T[0]+=x[0];T[1]+=x[1];T[2]+=x[2]
print('ALL',T[1],'status',T[2],'full',T[0],'%.1f%%'%(100*T[0]/T[1]))
print('\nPER FIELD (cells where both status=found; missing on one side = differ)')
for (v,f),(ok,n) in sorted(fld.items()): print(v,f,'%d/%d'%(ok,n))
print('\nSTATUS PAIRS');print(collections.Counter((R1[k]['status'],R2[k]['status']) for k in keys))
print('\nDISAGREEMENTS',len(dis))
for k,a,b,ca,cb,sa in dis:
    print('\n###',k[0],k[1],'r1',FR1+':%d'%a['_l'],'r2',FR2+':%d'%b['_l'])
    print(' r1 status=%s pages=[%s] basis=[%s] value=%s'%(a['status'],a['pages'],a['basis'],a['value']))
    print(' r2 status=%s pages=[%s] basis=[%s] value=%s'%(b['status'],b['pages'],b['basis'],b['value']))
    print(' differing compared fields:',{f:(ca.get(f),cb.get(f)) for f in set(ca)|set(cb) if ca.get(f)!=cb.get(f)} ,'' if sa else 'STATUS DIFFERS')
