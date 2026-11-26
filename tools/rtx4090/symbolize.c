/* symbolize.c - resolve sampler stack frames via dbghelp, print self/inclusive rankings.
 * Build: cl /O2 /W4 symbolize.c /Fe:symbolize.exe /link dbghelp.lib
 * Usage: symbolize.exe <profile.txt> <out.txt> <module.exe> <pdb-search-dir>
 */
#include <windows.h>
#include <dbghelp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <ctype.h>

#define LINEBUF 65536

typedef struct { const char *name; unsigned long long count; } Cent;
typedef struct { unsigned long long rva; const char *name; } Rent;
typedef struct { const char *thread; long long count; int nf; const char **fr; } Stack;
typedef struct { char *id; unsigned long long samples; char *raw; } Thread;

static HANDLE   g_proc;
static DWORD64  g_base;
static char     g_modname[MAX_PATH];

/* ---------- string-keyed counter / intern table ---------- */
static Cent *ctab; static size_t ccap, cused;

static size_t hstr(const char *s){ size_t h=1469598103934665603ULL;
    while(*s){ h^=(unsigned char)*s++; h*=1099511628211ULL; } return h; }

static Cent *cent_slot(const char *name){
    size_t mask=ccap-1,i=hstr(name)&mask;
    while(ctab[i].name){ if(!strcmp(ctab[i].name,name)) return &ctab[i]; i=(i+1)&mask; }
    return &ctab[i];
}
static void cent_grow(void){
    Cent *old=ctab; size_t ocap=ccap;
    ccap=ccap?ccap*2:1<<14; ctab=calloc(ccap,sizeof(Cent)); cused=0;
    if(old){ size_t i; for(i=0;i<ocap;i++) if(old[i].name){ Cent*d=cent_slot(old[i].name); *d=old[i]; cused++; } free(old); }
}
static Cent *cent_get(const char *name){
    if(!ccap||cused*4>=ccap*3) cent_grow();
    Cent*c=cent_slot(name);
    if(!c->name){ c->name=_strdup(name); c->count=0; cused++; }
    return c;
}
static void cent_reset(void){ if(ctab) memset(ctab,0,ccap*sizeof(Cent)); cused=0; }

/* ---------- rva -> symbol-name cache ---------- */
static Rent *rtab; static size_t rcap,rused;

static size_t rva_slot(unsigned long long rva){
    size_t mask=rcap-1,i=(size_t)((rva*0x9E3779B97F4A7C15ULL)>>32)&mask;
    while(rtab[i].name){ if(rtab[i].rva==rva) break; i=(i+1)&mask; }
    return i;
}
static const char *rva_find(unsigned long long rva){
    size_t i; if(!rcap) return NULL; i=rva_slot(rva);
    return rtab[i].name&&rtab[i].rva==rva ? rtab[i].name : NULL;
}
static void rva_put(unsigned long long rva,const char*name){
    if(!rcap||rused*4>=rcap*3){
        Rent*old=rtab; size_t ocap=rcap;
        rcap=rcap?rcap*2:1<<12; rtab=calloc(rcap,sizeof(Rent)); rused=0;
        if(old){ size_t i; for(i=0;i<ocap;i++) if(old[i].name){ Rent*d=&rtab[rva_slot(old[i].rva)]; *d=old[i]; rused++; } free(old); }
    }
    { Rent*d=&rtab[rva_slot(rva)]; d->rva=rva; d->name=name; rused++; }
}

static const char *sym_resolve(unsigned long long rva){
    char buf[sizeof(SYMBOL_INFO)+MAX_SYM_NAME],tmp[MAX_SYM_NAME+64];
    PSYMBOL_INFO si=(PSYMBOL_INFO)buf;
    const char*c=rva_find(rva); if(c) return c;
    memset(buf,0,sizeof buf);
    si->SizeOfStruct=sizeof(SYMBOL_INFO); si->MaxNameLen=MAX_SYM_NAME;
    if(SymFromAddr(g_proc,g_base+rva,NULL,si)) _snprintf(tmp,sizeof tmp,"%s",si->Name);
    else { static int once=0; if(!once++){ IMAGEHLP_MODULE64 mi; memset(&mi,0,sizeof mi); mi.SizeOfStruct=sizeof mi; SymGetModuleInfo64(g_proc,g_base,&mi); fprintf(stderr,"SymFromAddr gle=%lu symtype=%d pdb=%s\n",(unsigned long)GetLastError(),(int)mi.SymType,mi.LoadedPdbName);} _snprintf(tmp,sizeof tmp,"%s+%llx",g_modname,(unsigned long long)rva); }
    tmp[sizeof tmp-1]=0;
    c=_strdup(tmp); rva_put(rva,c); return c;
}
/* collapse foreign module frames to the module name; intern to avoid dup strings */
static const char *mod_intern(const char*mb){
    static char*names[1024]; static int nn=0; int i;
    for(i=0;i<nn;i++) if(!strcmp(names[i],mb)) return names[i];
    if(nn<1024) return names[nn++]=_strdup(mb);
    return _strdup(mb);
}
static const char *resolve_frame(const char*tok){
    const char*plus=strchr(tok,'+');
    char mb[MAX_PATH]; size_t ml; unsigned long long rva;
    if(!plus) return _strdup(tok);
    ml=(size_t)(plus-tok); if(ml>=MAX_PATH) ml=MAX_PATH-1;
    memcpy(mb,tok,ml); mb[ml]=0; { char*p; for(p=mb;*p;p++)*p=(char)tolower((unsigned char)*p); }
    rva=strtoull(plus+1,NULL,16);            /* accepts optional 0x prefix */
    if(!strcmp(mb,g_modname)) return sym_resolve(rva);
    return mod_intern(mb);
}

/* one ranking: self (innermost frame) or inclusive (distinct names per stack) */
static void rank_to(FILE*out,Stack*st,size_t ns,const char*thread,int self,int topn){
    unsigned long long total=0; Cent*arr; size_t i,j=0,n;
    cent_reset();
    for(i=0;i<ns;i++){
        int k;
        if(thread&&strcmp(st[i].thread,thread)) continue;
        if(!st[i].nf||st[i].count<=0) continue;
        if(self){ Cent*c=cent_get(st[i].fr[0]); c->count+=(unsigned long long)st[i].count; }
        else for(k=0;k<st[i].nf;k++){
            Cent*c=cent_get(st[i].fr[k]); int d,seen=0;
            for(d=0;d<k;d++) if(st[i].fr[d]==st[i].fr[k]){ seen=1; break; }
            if(seen) continue;
            c->count+=(unsigned long long)st[i].count;
        }
    }
    arr=malloc((cused?cused:1)*sizeof(Cent)); if(!arr){ fprintf(stderr,"out of memory\n"); exit(1); }
    for(i=0;i<ccap;i++) if(ctab[i].name){ total+=ctab[i].count; arr[j++]=ctab[i]; }
    n=j;
    { int centcmp(const void*,const void*);
      qsort(arr,n,sizeof(Cent),centcmp); }
    for(i=0;i<n&&i<(size_t)topn;i++){
        if(!arr[i].count) continue;
        fprintf(out,"%llu\t%.2f\t%s\n",arr[i].count,total?100.0*(double)arr[i].count/(double)total:0.0,arr[i].name);
    }
    free(arr);
}
int centcmp(const void*a,const void*b){
    const Cent*x=(const Cent*)a,*y=(const Cent*)b;
    if(x->count>y->count) return -1;
    if(x->count<y->count) return  1;
    return strcmp(x->name,y->name);
}

int main(int argc,char**argv){
    const char*inpath,*outpath,*modpath,*pdbdir;
    const char*b1,*b2,*b;
    FILE*in,*out;
    static char line[LINEBUF],fbuf[LINEBUF];
    Stack*stacks=NULL; size_t ns=0,cap=0;
    Thread*th=NULL; size_t nth=0,cth=0,i;
    size_t chosen[3]; int nch;

    if(argc!=5){ fprintf(stderr,"usage: %s <profile.txt> <out.txt> <module.exe> <pdb-dir>\n",argv[0]); return 1; }
    inpath=argv[1]; outpath=argv[2]; modpath=argv[3]; pdbdir=argv[4];

    b1=strrchr(modpath,'/'); b2=strrchr(modpath,'\\'); b=(b1>b2)?b1:b2;
    b=b?b+1:modpath;
    strncpy(g_modname,b,MAX_PATH-1); g_modname[MAX_PATH-1]=0;
    { char*p; for(p=g_modname;*p;p++)*p=(char)tolower((unsigned char)*p); }

    in=fopen(inpath,"rb");
    if(!in){ fprintf(stderr,"cannot open %s\n",inpath); return 1; }

    g_proc=GetCurrentProcess();
    SymSetOptions(SYMOPT_UNDNAME|SYMOPT_LOAD_LINES|SYMOPT_LOAD_ANYTHING|SYMOPT_FAIL_CRITICAL_ERRORS);
    if(!SymInitialize(g_proc,pdbdir,FALSE)){
        fprintf(stderr,"SymInitialize failed (gle=%lu)\n",(unsigned long)GetLastError()); fclose(in); return 1;
    }
    g_base=SymLoadModuleEx(g_proc,NULL,modpath,NULL,0x140000000,0,NULL,0);
    if(!g_base){
        fprintf(stderr,"SymLoadModuleEx failed for %s (gle=%lu)\n",modpath,(unsigned long)GetLastError());
        SymCleanup(g_proc); fclose(in); return 1;
    }

    while(fgets(line,sizeof line,in)){
        size_t len=strlen(line);
        char *ctx,*tok,*id=NULL,*cnt=NULL,*frames=NULL; int f=-1;
        while(len&&(line[len-1]=='\n'||line[len-1]=='\r')) line[--len]=0;
        if(!len) continue;
        if(!strncmp(line,"thread ",7)){
            char*raw=_strdup(line); char*samp=NULL; int fld=-1;
            for(tok=strtok_s(line," ",&ctx);tok;tok=strtok_s(NULL," ",&ctx)){
                fld++; if(fld==1)id=tok; else if(fld==4)samp=tok;
            }
            if(id&&samp){
                Thread*t=NULL; size_t j;
                for(j=0;j<nth;j++) if(!strcmp(th[j].id,id)){ t=&th[j]; break; }
                if(t){ t->samples=strtoull(samp,NULL,10); free(t->raw); t->raw=raw; }
                else{
                    if(nth==cth){ cth=cth?cth*2:16; th=realloc(th,cth*sizeof(Thread));
                                  if(!th){ fprintf(stderr,"out of memory\n"); return 1; } }
                    th[nth].id=_strdup(id); th[nth].samples=strtoull(samp,NULL,10); th[nth].raw=raw; nth++;
                }
            } else free(raw);
            continue;
        }
        if(!strncmp(line,"stack ",6)){
            char*tid=NULL,*fs; long long cntv; const char**fr=NULL; int nf=0,capf=0;
            for(tok=strtok_s(line," ",&ctx);tok;tok=strtok_s(NULL," ",&ctx)){
                f++; if(f==1)id=tok; else if(f==3)cnt=tok; else if(f==4){ frames=tok; break; }
            }
            if(!id||!cnt||!frames) continue;
            { size_t j; for(j=0;j<nth;j++) if(!strcmp(th[j].id,id)){ tid=th[j].id; break; } }
            if(!tid) tid=_strdup(id);
            cntv=strtol(cnt,NULL,10);
            { size_t flen=strlen(frames); if(flen>=LINEBUF) flen=LINEBUF-1; memcpy(fbuf,frames,flen); fbuf[flen]=0; }
            fs=fbuf;
            for(tok=strtok_s(fs,";",&ctx);tok;tok=strtok_s(NULL,";",&ctx)){
                const char*nm; if(!*tok) continue;
                nm=resolve_frame(tok);
                if(nf==capf){ capf=capf?capf*2:16; fr=realloc(fr,capf*sizeof(char*));
                              if(!fr){ fprintf(stderr,"out of memory\n"); return 1; } }
                fr[nf++]=nm;
            }
            if(ns==cap){ cap=cap?cap*2:64; stacks=realloc(stacks,cap*sizeof(Stack));
                         if(!stacks){ fprintf(stderr,"out of memory\n"); return 1; } }
            stacks[ns].thread=tid; stacks[ns].count=cntv; stacks[ns].fr=fr; stacks[ns].nf=nf; ns++;
            { static FILE*fold=NULL; if(!fold){ char fn[MAX_PATH]; _snprintf(fn,sizeof fn,"%s.folded",argv[2]); fn[MAX_PATH-1]=0; fold=fopen(fn,"w"); }
              if(fold){ int k; fprintf(fold,"%s %lld ",tid,cntv); for(k=0;k<nf;k++) fprintf(fold,"%s%s",k?";":"",fr[k]); fprintf(fold,"\n"); } }
        }
    }
    fclose(in);

    out=fopen(outpath,"w");
    if(!out){ fprintf(stderr,"cannot write %s\n",outpath); return 1; }

    fprintf(out,"SELF all\n");    rank_to(out,stacks,ns,NULL,1,60);
    fprintf(out,"INCLUSIVE all\n"); rank_to(out,stacks,ns,NULL,0,80);

    for(nch=0;nch<3;nch++){
        size_t best=(size_t)-1; unsigned long long bv=0; int j,taken;
        for(i=0;i<nth;i++){
            taken=0; for(j=0;j<nch;j++) if(chosen[j]==i){ taken=1; break; }
            if(!taken&&th[i].samples>bv){ bv=th[i].samples; best=i; }
        }
        if(best==(size_t)-1) break;
        chosen[nch]=best;
        fprintf(out,"%s\n",th[best].raw?th[best].raw:"");
        fprintf(out,"SELF %s\n",th[best].id);    rank_to(out,stacks,ns,th[best].id,1,60);
        fprintf(out,"INCLUSIVE %s\n",th[best].id); rank_to(out,stacks,ns,th[best].id,0,80);
    }

    fclose(out);
    SymCleanup(g_proc);
    return 0;
}
