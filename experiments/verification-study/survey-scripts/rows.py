import csv
def rows(path):
    """yield (physical start line, dict) so line numbers match `grep -n`/editors"""
    with open(path,newline='') as fh:
        rd=csv.reader(fh); hdr=next(rd); prev=rd.line_num
        for rec in rd:
            yield prev+1, dict(zip(hdr,rec)); prev=rd.line_num
