import re,sys,datetime as dt
run,pin=sys.argv[1],sys.argv[2]
ev=[]
for l in open(pin):
    p=l.split()
    if len(p)>=2: ev.append((dt.datetime.fromisoformat(p[0]),p[1]))
rx=re.compile(r'^(\S+)Z.*victory_gpu::host: ([0-9.]+) fps, ([0-9.]+) ms/frame \(([0-9.]+) preparing.*?([0-9]+) draws/frame')
st={'on':[], 'off':[]}
for l in open(run,encoding='utf-8',errors='replace'):
    l=re.sub(r'\x1b\[[0-9;]*m','',l); m=rx.match(l)
    if not m or int(m.group(5))<1000: continue
    t=dt.datetime.fromisoformat(m.group(1)[:26])
    cur=None
    for et,s in ev:
        if et<=t: cur=(et,s)
    if not cur or (t-cur[0]).total_seconds()<6: continue
    st[cur[1].rstrip(':')].append((float(m.group(2)),float(m.group(4))))
for k,v in st.items():
    if v: print(k,len(v),'fps %.2f'%(sum(a for a,_ in v)/len(v)),'prep %.1f'%(sum(b for _,b in v)/len(v)))
