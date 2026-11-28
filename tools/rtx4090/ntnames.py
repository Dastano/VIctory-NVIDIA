import struct,sys
d=open(sys.argv[1],'rb').read()
pe=struct.unpack_from('<I',d,0x3c)[0]; nsec=struct.unpack_from('<H',d,pe+6)[0]; opt=pe+24; osz=struct.unpack_from('<H',d,pe+20)[0]
secs=[(struct.unpack_from('<I',d,opt+osz+i*40+12)[0],struct.unpack_from('<I',d,opt+osz+i*40+8)[0],struct.unpack_from('<I',d,opt+osz+i*40+20)[0]) for i in range(nsec)]
def off(r):
    for va,vs,raw in secs:
        if va<=r<va+vs: return r-va+raw
e=off(struct.unpack_from('<I',d,opt+112)[0]); nf,nn,af,an,ao=struct.unpack_from('<IIIII',d,e+20)
funcs=[struct.unpack_from('<I',d,off(af)+4*i)[0] for i in range(nf)]; names={}
for i in range(nn):
    nr=struct.unpack_from('<I',d,off(an)+4*i)[0]; o=struct.unpack_from('<H',d,off(ao)+2*i)[0]; names.setdefault(funcs[o],d[off(nr):d.index(b'\0',off(nr))].decode())
fs=sorted(names.items())
for t in sys.argv[2:]:
    t=int(t,16); print(hex(t),[(hex(r),n) for r,n in fs if t<=r<t+0x100])
