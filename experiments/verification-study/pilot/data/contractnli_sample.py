import json, random, collections, sys, os
SRC='/home/user/study/data/contractnli_raw/x/contract-nli/'
OUT='/home/user/study/data/contractnli/'
os.makedirs(OUT,exist_ok=True)
LABELS=['Entailment','Contradiction','NotMentioned']
def pairs(split):
    d=json.load(open(SRC+split+'.json'))
    for doc in d['documents']:
        ann=doc['annotation_sets'][0]['annotations']
        for k,a in ann.items():
            yield dict(doc_id=doc['id'],file_name=doc['file_name'],document_type=doc['document_type'],
              hypothesis_id=k,hypothesis=d['labels'][k]['hypothesis'],gold_label=a['choice'],
              gold_evidence_span_ids=a['spans'],
              gold_evidence_spans=[[doc['spans'][i][0],doc['spans'][i][1],doc['text'][doc['spans'][i][0]:doc['spans'][i][1]]] for i in a['spans']],
              document_text=doc['text'],n_words=len(doc['text'].split()))
def draw(split,seed,counts,fname):
    allp=list(pairs(split)); rng=random.Random(seed); out=[]
    for lab,n in zip(LABELS,counts):
        pool=sorted([p for p in allp if p['gold_label']==lab],key=lambda p:(p['doc_id'],p['hypothesis_id']))
        out+=rng.sample(pool,n)
    # at most one pair per document would be nicer; enforce by redraw check
    rng.shuffle(out)
    with open(OUT+fname,'w') as f:
        for i,p in enumerate(out): p['item_id']=f'{split}-{i:02d}'; f.write(json.dumps(p)+'\n')
    print(fname,collections.Counter(p['gold_label'] for p in out),'docs',len({p['doc_id'] for p in out}))
if __name__=='__main__':
    draw('test',20211005,(14,13,13),'sample40.jsonl')
    draw('dev',20240607,(4,3,3),'dev10.jsonl')
    # stats
    for s in ['train','dev','test']:
        d=json.load(open(SRC+s+'.json')); ps=list(pairs(s))
        w=sorted(len(x['text'].split()) for x in d['documents'])
        print(s,'docs',len(d['documents']),'pairs',len(ps),collections.Counter(p['gold_label'] for p in ps),
              'words min/med/mean/max',w[0],w[len(w)//2],sum(w)//len(w),w[-1])
