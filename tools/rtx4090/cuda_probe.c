/* cuda_probe.c -- probe cuMemHostRegister behaviour on Windows x64.
   Build: cl /O2 /W4 /std:c11 cuda_probe.c  (no CUDA SDK needed; loads nvcuda.dll) */
#include <windows.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef int CUresult;
typedef int CUdevice;
typedef void *CUcontext;
typedef unsigned long long CUdeviceptr;

typedef CUresult (*fnInit)(unsigned);
typedef CUresult (*fnDevGet)(CUdevice *, int);
typedef CUresult (*fnCtxCreate)(CUcontext *, unsigned, CUdevice);
typedef CUresult (*fnHostReg)(void *, size_t, unsigned);
typedef CUresult (*fnHostUnreg)(void *);
typedef CUresult (*fnMemAlloc)(CUdeviceptr *, size_t);
typedef CUresult (*fnHtoD)(CUdeviceptr, const void *, size_t);
typedef CUresult (*fnDtoH)(void *, CUdeviceptr, size_t);
typedef CUresult (*fnMemFree)(CUdeviceptr);
typedef CUresult (*fnCtxDestroy)(CUcontext);
typedef CUresult (*fnErrStr)(CUresult, const char **);

#define NBYTES  (64ull << 20)
#define PORTABLE 1u
#define DEVICEMAP 2u
#define READONLY 8u

static struct {
    fnInit Init;
    fnDevGet DeviceGet;
    fnCtxCreate CtxCreate;
    fnHostReg HostReg;
    fnHostUnreg HostUnreg;
    fnMemAlloc MemAlloc;
    fnHtoD MemcpyHtoD;
    fnDtoH MemcpyDtoH;
    fnMemFree MemFree;
    fnCtxDestroy CtxDestroy;
    fnErrStr GetErrorString;
} cu;

static double g_freq;
static void *g_va; /* case-1 buffer, kept alive for case 6, freed by main */

static const char *es(CUresult r)
{
    const char *s = "unknown";
    cu.GetErrorString(r, &s);
    return s;
}

static void touch(void *p)
{
    size_t i;
    volatile char *q = (volatile char *)p;
    for (i = 0; i < NBYTES; i += 4096)
        q[i] = 1;
    q[NBYTES - 1] = 1;
}

/* 5 timed HtoD copies, returns best ms; *ok=0 on any failure */
static double best_htod(CUdeviceptr dst, const void *src, int *ok)
{
    LARGE_INTEGER a, b;
    double best = 1e30;
    int i;
    *ok = 1;
    for (i = 0; i < 5; i++) {
        CUresult r;
        double ms;
        QueryPerformanceCounter(&a);
        r = cu.MemcpyHtoD(dst, src, (size_t)NBYTES);
        QueryPerformanceCounter(&b);
        if (r) {
            printf("  cuMemcpyHtoD_v2 failed rc=%d (%s)\n", r, es(r));
            *ok = 0;
            return 0;
        }
        ms = (double)(b.QuadPart - a.QuadPart) * 1000.0 / g_freq;
        if (ms < best)
            best = ms;
    }
    return best;
}

static void report(const char *name, CUresult rreg, double ms)
{
    printf("case %s: register=%d %s, copy 64MB = %.3f ms (%.2f GB/s)\n",
           name, rreg, es(rreg), ms, (double)NBYTES / (ms * 1e6));
}

/* Case 1. Keeps its allocation in g_va for the memcpy baseline. */
static void case_virtualalloc(CUdeviceptr dst)
{
    const char *name = "virtualalloc";
    void *p;
    CUresult rr;
    int ok = 0;

    p = VirtualAlloc(NULL, (SIZE_T)NBYTES, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if (!p) {
        printf("case %s: FAILED VirtualAlloc gle=%lu\n", name, GetLastError());
        return;
    }
    g_va = p;
    touch(p);
    rr = cu.HostReg(p, (size_t)NBYTES, 0);
    if (rr) {
        printf("case %s: FAILED cuMemHostRegister rc=%d (%s)\n", name, rr, es(rr));
        VirtualFree(p, 0, MEM_RELEASE);
        g_va = NULL;
        return;
    }
    (void)best_htod(dst, p, &ok); /* failures printed inside */
    if (ok)
        report(name, rr, best_htod(dst, p, &ok));
    else {
        CUresult ur = cu.HostUnreg(p);
        if (ur)
            printf("  cuMemHostUnregister failed rc=%d (%s)\n", ur, es(ur));
        VirtualFree(p, 0, MEM_RELEASE);
        g_va = NULL;
    }
    /* on success: unregister now, keep allocation for case memcpy-baseline */
    if (ok) {
        CUresult ur = cu.HostUnreg(p);
        if (ur)
            printf("  cuMemHostUnregister failed rc=%d (%s)\n", ur, es(ur));
    }
}

/* Cases 2-4: named section, SEC_COMMIT or SEC_RESERVE(+VirtualAlloc commit). */
static void case_section(int reserve, unsigned regflags, const char *name, CUdeviceptr dst)
{
    HANDLE h = NULL;
    void *v = NULL;
    CUresult rr = -1;
    int reg = 0, ok = 0;

    h = CreateFileMappingA(INVALID_HANDLE_VALUE, NULL,
                           PAGE_READWRITE | (reserve ? SEC_RESERVE : SEC_COMMIT),
                           0, (DWORD)NBYTES, NULL);
    if (!h) {
        printf("case %s: FAILED CreateFileMappingA gle=%lu\n", name, GetLastError());
        return;
    }
    v = MapViewOfFile(h, FILE_MAP_ALL_ACCESS, 0, 0, (SIZE_T)NBYTES);
    if (!v) {
        printf("case %s: FAILED MapViewOfFile gle=%lu\n", name, GetLastError());
        goto cleanup;
    }
    if (reserve && !VirtualAlloc(v, (SIZE_T)NBYTES, MEM_COMMIT, PAGE_READWRITE)) {
        printf("case %s: FAILED VirtualAlloc(MEM_COMMIT) in view gle=%lu\n", name, GetLastError());
        goto cleanup;
    }
    touch(v);
    rr = cu.HostReg(v, (size_t)NBYTES, regflags);
    if (rr) {
        printf("case %s: FAILED cuMemHostRegister rc=%d (%s)\n", name, rr, es(rr));
        goto cleanup;
    }
    reg = 1;
    best_htod(dst, v, &ok);
    if (ok)
        report(name, rr, best_htod(dst, v, &ok));

cleanup:
    if (reg) {
        CUresult ur = cu.HostUnreg(v);
        if (ur)
            printf("  cuMemHostUnregister failed rc=%d (%s)\n", ur, es(ur));
    }
    if (v)
        UnmapViewOfFile(v);
    if (h)
        CloseHandle(h);
}

/* Case 5: VirtualAlloc, protect PAGE_READONLY, register, copy, restore. */
static void case_guarded(CUdeviceptr dst)
{
    const char *name = "guarded";
    void *p;
    DWORD old = 0;
    CUresult rr;
    int ok = 0;

    p = VirtualAlloc(NULL, (SIZE_T)NBYTES, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if (!p) {
        printf("case %s: FAILED VirtualAlloc gle=%lu\n", name, GetLastError());
        return;
    }
    touch(p);
    if (!VirtualProtect(p, (SIZE_T)NBYTES, PAGE_READONLY, &old)) {
        printf("case %s: FAILED VirtualProtect(PAGE_READONLY) gle=%lu\n", name, GetLastError());
        VirtualFree(p, 0, MEM_RELEASE);
        return;
    }
    rr = cu.HostReg(p, (size_t)NBYTES, 0);
    if (rr) {
        printf("case %s: FAILED cuMemHostRegister rc=%d (%s)\n", name, rr, es(rr));
        VirtualFree(p, 0, MEM_RELEASE);
        return;
    }
    best_htod(dst, p, &ok);
    if (ok)
        report(name, rr, best_htod(dst, p, &ok));
    if (VirtualProtect(p, (SIZE_T)NBYTES, PAGE_READWRITE, &old))
        printf("  VirtualProtect back to PAGE_READWRITE: succeeded\n");
    else
        printf("  VirtualProtect back to PAGE_READWRITE: FAILED gle=%lu\n", GetLastError());
    {
        CUresult ur = cu.HostUnreg(p);
        if (ur)
            printf("  cuMemHostUnregister failed rc=%d (%s)\n", ur, es(ur));
    }
    VirtualFree(p, 0, MEM_RELEASE);
}

/* Case 7: two views of one section; register viewB, flip protection on
   viewA, write through viewA, verify viewB sees it, time the copy. */
static void case_second_view(CUdeviceptr dst)
{
    const char *name = "second-view";
    HANDLE h = NULL;
    void *va = NULL, *vb = NULL;
    unsigned char *host = NULL;
    DWORD old = 0;
    CUresult rr, rd;
    int reg = 0, ok = 0;
    double ms;
    size_t i, mismatches = 0;

    h = CreateFileMappingA(INVALID_HANDLE_VALUE, NULL, PAGE_READWRITE | SEC_RESERVE,
                           0, (DWORD)NBYTES, NULL);
    if (!h) {
        printf("case %s: FAILED CreateFileMappingA gle=%lu\n", name, GetLastError());
        return;
    }
    va = MapViewOfFile(h, FILE_MAP_ALL_ACCESS, 0, 0, (SIZE_T)NBYTES);
    if (!va) {
        printf("case %s: FAILED MapViewOfFile(viewA) gle=%lu\n", name, GetLastError());
        goto done;
    }
    vb = MapViewOfFile(h, FILE_MAP_ALL_ACCESS, 0, 0, (SIZE_T)NBYTES);
    if (!vb) {
        printf("case %s: FAILED MapViewOfFile(viewB) gle=%lu\n", name, GetLastError());
        goto done;
    }
    if (!VirtualAlloc(va, (SIZE_T)NBYTES, MEM_COMMIT, PAGE_READWRITE)) {
        printf("case %s: FAILED VirtualAlloc(MEM_COMMIT) in viewA gle=%lu\n", name, GetLastError());
        goto done;
    }
    memset(va, 0x11, (size_t)NBYTES);
    rr = cu.HostReg(vb, (size_t)NBYTES, 0);
    printf("case %s: cuMemHostRegister(viewB) rc=%d (%s)\n", name, rr, es(rr));
    if (rr)
        goto done;
    reg = 1;
    if (!VirtualProtect(va, (SIZE_T)NBYTES, PAGE_READONLY, &old))
        printf("case %s: VirtualProtect(viewA, PAGE_READONLY): FAILED gle=%lu\n", name, GetLastError());
    else
        printf("case %s: VirtualProtect(viewA, PAGE_READONLY): succeeded\n", name);
    if (!VirtualProtect(va, (SIZE_T)NBYTES, PAGE_READWRITE, &old)) {
        printf("case %s: FAILED VirtualProtect(viewA, PAGE_READWRITE) gle=%lu\n", name, GetLastError());
        goto done;
    }
    {
        volatile unsigned char *q = (volatile unsigned char *)va;
        for (i = 0; i < NBYTES; i += 4096)
            q[i] = 0x22;
    }
    ms = best_htod(dst, vb, &ok);
    if (!ok)
        goto done;
    host = malloc((size_t)NBYTES);
    if (!host) {
        printf("case %s: FAILED malloc\n", name);
        goto done;
    }
    rd = cu.MemcpyDtoH(host, dst, (size_t)NBYTES);
    if (rd) {
        printf("  cuMemcpyDtoH_v2 failed rc=%d (%s)\n", rd, es(rd));
        goto done;
    }
    for (i = 0; i < NBYTES; i += 4096)
        if (host[i] != 0x22)
            mismatches++;
    printf("case %s: coherent=%s, mismatched pages=%lu of %lu\n", name,
           mismatches ? "no" : "yes", (unsigned long)mismatches,
           (unsigned long)(NBYTES / 4096));
    printf("case %s: copy 64MB = %.3f ms (%.2f GB/s)\n", name, ms,
           (double)NBYTES / (ms * 1e6));

done:
    free(host);
    if (reg) {
        CUresult ur = cu.HostUnreg(vb);
        if (ur)
            printf("  cuMemHostUnregister failed rc=%d (%s)\n", ur, es(ur));
    }
    if (vb)
        UnmapViewOfFile(vb);
    if (va)
        UnmapViewOfFile(va);
    if (h)
        CloseHandle(h);
}

/* Case 6: plain CPU memcpy, src = case-1 buffer. */
static void case_memcpy_baseline(void)
{
    const char *name = "memcpy-baseline";
    int own = 0;
    void *dst;
    LARGE_INTEGER a, b;
    double best = 1e30;
    int i;

    if (!g_va) {
        g_va = VirtualAlloc(NULL, (SIZE_T)NBYTES, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        if (!g_va) {
            printf("case %s: FAILED VirtualAlloc fallback gle=%lu\n", name, GetLastError());
            return;
        }
        touch(g_va);
        own = 1;
    }
    dst = malloc((size_t)NBYTES);
    if (!dst) {
        printf("case %s: FAILED malloc\n", name);
        if (own) {
            VirtualFree(g_va, 0, MEM_RELEASE);
            g_va = NULL;
        }
        return;
    }
    for (i = 0; i < 5; i++) {
        double ms;
        QueryPerformanceCounter(&a);
        memcpy(dst, g_va, (size_t)NBYTES);
        QueryPerformanceCounter(&b);
        ms = (double)(b.QuadPart - a.QuadPart) * 1000.0 / g_freq;
        if (ms < best)
            best = ms;
    }
    printf("case %s: copy 64MB = %.3f ms (%.2f GB/s)\n", name, best,
           (double)NBYTES / (best * 1e6));
    free(dst);
    if (own) {
        VirtualFree(g_va, 0, MEM_RELEASE);
        g_va = NULL;
    }
}

int main(void)
{
    LARGE_INTEGER f;
    HMODULE hd;
    CUresult r;
    CUdevice dev = 0;
    CUcontext ctx = NULL;
    CUdeviceptr dst = 0;

    QueryPerformanceFrequency(&f);
    g_freq = (double)f.QuadPart;

    hd = LoadLibraryA("nvcuda.dll");
    if (!hd) {
        printf("FAILED LoadLibraryA(nvcuda.dll) gle=%lu\n", GetLastError());
        return 0;
    }
#define GET(field, type, fname)                                              \
    do {                                                                     \
        cu.field = (type)GetProcAddress(hd, fname);                          \
        if (!cu.field) {                                                     \
            printf("FAILED GetProcAddress(%s) gle=%lu\n", fname, GetLastError()); \
            return 0;                                                        \
        }                                                                    \
    } while (0)
    GET(Init, fnInit, "cuInit");
    GET(DeviceGet, fnDevGet, "cuDeviceGet");
    GET(CtxCreate, fnCtxCreate, "cuCtxCreate_v2");
    GET(HostReg, fnHostReg, "cuMemHostRegister_v2");
    GET(HostUnreg, fnHostUnreg, "cuMemHostUnregister");
    GET(MemAlloc, fnMemAlloc, "cuMemAlloc_v2");
    GET(MemcpyHtoD, fnHtoD, "cuMemcpyHtoD_v2");
    GET(MemcpyDtoH, fnDtoH, "cuMemcpyDtoH_v2");
    GET(MemFree, fnMemFree, "cuMemFree_v2");
    GET(CtxDestroy, fnCtxDestroy, "cuCtxDestroy_v2");
    GET(GetErrorString, fnErrStr, "cuGetErrorString");
#undef GET

    r = cu.Init(0);
    if (r) {
        printf("cuInit failed rc=%d (%s)\n", r, es(r));
        return 0;
    }
    r = cu.DeviceGet(&dev, 0);
    if (r) {
        printf("cuDeviceGet failed rc=%d (%s)\n", r, es(r));
        return 0;
    }
    r = cu.CtxCreate(&ctx, 0, dev);
    if (r) {
        printf("cuCtxCreate_v2 failed rc=%d (%s)\n", r, es(r));
        return 0;
    }
    r = cu.MemAlloc(&dst, (size_t)NBYTES);
    if (r) {
        printf("cuMemAlloc_v2 failed rc=%d (%s)\n", r, es(r));
        cu.CtxDestroy(ctx);
        return 0;
    }

    case_virtualalloc(dst);
    case_section(0, 0, "section-commit", dst);
    case_section(1, 0, "section-reserve", dst);
    case_section(1, READONLY, "section-reserve-readonly", dst);
    case_guarded(dst);
    case_second_view(dst);
    case_memcpy_baseline();

    if (dst)
        cu.MemFree(dst);
    if (ctx)
        cu.CtxDestroy(ctx);
    if (g_va)
        VirtualFree(g_va, 0, MEM_RELEASE);
    FreeLibrary(hd);
    return 0;
}
