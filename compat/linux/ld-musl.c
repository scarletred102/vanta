#include <stdint.h>
#include <stddef.h>

#define AT_NULL   0
#define AT_PHDR   3
#define AT_PHENT  4
#define AT_PHNUM  5
#define AT_PAGESZ 6
#define AT_BASE   7
#define AT_FLAGS  8
#define AT_ENTRY  9
#define AT_RANDOM 25

#define PT_NULL    0
#define PT_LOAD    1
#define PT_DYNAMIC 2
#define PT_INTERP  3
#define PT_PHDR    6

#define DT_NULL     0
#define DT_NEEDED   1
#define DT_PLTRELSZ 2
#define DT_PLTGOT   3
#define DT_HASH     4
#define DT_STRTAB   5
#define DT_SYMTAB   6
#define DT_RELA     7
#define DT_RELASZ   8
#define DT_RELAENT  9
#define DT_STRSZ    10
#define DT_SYMENT   11
#define DT_JMPREL   23
#define DT_GNU_HASH 0x6ffffef5

#define R_X86_64_NONE      0
#define R_X86_64_64        1
#define R_X86_64_COPY      5
#define R_X86_64_GLOB_DAT  6
#define R_X86_64_JUMP_SLOT 7
#define R_X86_64_RELATIVE  8

#define ELF64_R_SYM(i)   ((uint32_t)((i) >> 32))
#define ELF64_R_TYPE(i)  ((uint32_t)((i) & 0xffffffffL))

#define PROT_READ  1
#define PROT_WRITE 2
#define PROT_EXEC  4

#define MAP_PRIVATE   0x02
#define MAP_FIXED     0x10
#define MAP_ANONYMOUS 0x20

typedef struct {
    unsigned char e_ident[16];
    uint16_t e_type;
    uint16_t e_machine;
    uint32_t e_version;
    uint64_t e_entry;
    uint64_t e_phoff;
    uint64_t e_shoff;
    uint32_t e_flags;
    uint16_t e_ehsize;
    uint16_t e_phentsize;
    uint16_t e_phnum;
    uint16_t e_shentsize;
    uint16_t e_shnum;
    uint16_t e_shstrndx;
} Elf64_Ehdr;

typedef struct {
    uint32_t p_type;
    uint32_t p_flags;
    uint64_t p_offset;
    uint64_t p_vaddr;
    uint64_t p_paddr;
    uint64_t p_filesz;
    uint64_t p_memsz;
    uint64_t p_align;
} Elf64_Phdr;

typedef struct {
    int64_t d_tag;
    union {
        uint64_t d_val;
        uint64_t d_ptr;
    } d_un;
} Elf64_Dyn;

typedef struct {
    uint32_t st_name;
    unsigned char st_info;
    unsigned char st_other;
    uint16_t st_shndx;
    uint64_t st_value;
    uint64_t st_size;
} Elf64_Sym;

typedef struct {
    uint64_t r_offset;
    uint64_t r_info;
    int64_t  r_addend;
} Elf64_Rela;

static int sys_write(int fd, const void *buf, size_t count) {
    int ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(1), "D"(fd), "S"(buf), "d"(count)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static int sys_open(const char *path, int flags) {
    int ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(2), "D"(path), "S"(flags)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static int sys_close(int fd) {
    int ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(3), "D"(fd)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static int sys_read(int fd, void *buf, size_t count) {
    int ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(0), "D"(fd), "S"(buf), "d"(count)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static int64_t sys_lseek(int fd, int64_t offset, int whence) {
    int64_t ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(8), "D"(fd), "S"(offset), "d"(whence)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static void *sys_mmap(void *addr, size_t length, int prot, int flags, int fd, int64_t offset) {
    void *ret;
    register int64_t r10 __asm__("r10") = flags;
    register int64_t r8  __asm__("r8")  = fd;
    register int64_t r9  __asm__("r9")  = offset;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(9), "D"(addr), "S"(length), "d"(prot), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static void sys_exit(int code) {
    __asm__ volatile (
        "syscall"
        :
        : "a"(60), "D"(code)
        : "rcx", "r11", "memory"
    );
}

static size_t my_strlen(const char *s) {
    size_t len = 0;
    while (s[len]) len++;
    return len;
}

static int my_strcmp(const char *a, const char *b) {
    while (*a && (*a == *b)) {
        a++;
        b++;
    }
    return *(const unsigned char *)a - *(const unsigned char *)b;
}

static void my_memset(void *dest, int val, size_t count) {
    unsigned char *d = (unsigned char *)dest;
    for (size_t i = 0; i < count; i++) {
        d[i] = (unsigned char)val;
    }
}

static void my_memcpy(void *dest, const void *src, size_t count) {
    unsigned char *d = (unsigned char *)dest;
    const unsigned char *s = (const unsigned char *)src;
    for (size_t i = 0; i < count; i++) {
        d[i] = s[i];
    }
}

static void log_str(const char *s) {
    sys_write(1, s, my_strlen(s));
}

static void log_hex(uint64_t val) {
    char buf[19];
    buf[0] = '0';
    buf[1] = 'x';
    for (int i = 15; i >= 0; i--) {
        int nibble = (val >> (i * 4)) & 0xf;
        buf[2 + (15 - i)] = nibble < 10 ? ('0' + nibble) : ('a' + nibble - 10);
    }
    buf[18] = 0;
    log_str(buf);
}

#define MAX_LOADED_LIBS 8
typedef struct {
    char name[64];
    uintptr_t base_addr;
    Elf64_Sym *symtab;
    const char *strtab;
    size_t sym_count;
} LoadedLib;

static LoadedLib loaded_libs[MAX_LOADED_LIBS];
static int loaded_lib_count = 0;
static uintptr_t next_lib_alloc = 0x7e0000000000ULL;

__attribute__((visibility("default")))
int __libc_start_main(int (*main)(int, char **, char **), int argc, char **argv) {
    log_str("[ldso] __libc_start_main: invoking application main()\n");
    char **envp = argv + argc + 1;
    int ret = main(argc, argv, envp);
    log_str("[ldso] __libc_start_main: main() returned, exiting\n");
    sys_exit(ret);
    return ret;
}

__attribute__((visibility("default")))
int write(int fd, const void *buf, size_t count) {
    return sys_write(fd, buf, count);
}

__attribute__((visibility("default")))
void exit(int status) {
    sys_exit(status);
}

static int sys_socket(int domain, int type, int protocol) {
    int ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(41), "D"(domain), "S"(type), "d"(protocol)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long sys_sendto(int fd, const void *buf, size_t len, int flags, const void *dest_addr, uint32_t addrlen) {
    long ret;
    register int64_t r10 __asm__("r10") = flags;
    register const void *r8  __asm__("r8")  = dest_addr;
    register uint64_t r9  __asm__("r9")  = addrlen;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(44), "D"(fd), "S"(buf), "d"(len), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long sys_recvfrom(int fd, void *buf, size_t len, int flags, void *src_addr, void *addrlen) {
    long ret;
    register int64_t r10 __asm__("r10") = flags;
    register void *r8  __asm__("r8")  = src_addr;
    register void *r9  __asm__("r9")  = addrlen;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(45), "D"(fd), "S"(buf), "d"(len), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory"
    );
    return ret;
}

struct my_sockaddr_in {
    uint16_t sin_family;
    uint16_t sin_port;
    uint32_t sin_addr;
    char     sin_zero[8];
};

struct my_addrinfo {
    int                    ai_flags;
    int                    ai_family;
    int                    ai_socktype;
    int                    ai_protocol;
    uint32_t               ai_addrlen;
    struct my_sockaddr_in *ai_addr;
    char                  *ai_canonname;
    struct my_addrinfo    *ai_next;
};

struct my_hostent {
    char  *h_name;
    char **h_aliases;
    int    h_addrtype;
    int    h_length;
    char **h_addr_list;
};

#define EAI_NONAME -2

static struct my_addrinfo g_addrinfo;
static struct my_sockaddr_in g_sockaddr;
static struct my_hostent g_hostent;
static char *g_addr_list[2];
static uint32_t g_host_addr;
static char g_host_name[128];

__attribute__((visibility("default")))
int getaddrinfo(const char *node, const char *service, const void *hints, struct my_addrinfo **res) {
    if (!node || !res) return EAI_NONAME;
    (void)service; (void)hints;

    // Build DNS query packet
    uint8_t qpkt[256];
    my_memset(qpkt, 0, sizeof(qpkt));
    qpkt[0] = 0x12; qpkt[1] = 0x34; // tx_id
    qpkt[2] = 0x01; qpkt[3] = 0x00; // flags: RD=1
    qpkt[4] = 0x00; qpkt[5] = 0x01; // QDCOUNT = 1

    int qpos = 12;
    const char *p = node;
    while (*p) {
        const char *dot = p;
        while (*dot && *dot != '.') dot++;
        int len = dot - p;
        if (len > 63 || qpos + 1 + len >= 250) return EAI_NONAME;
        qpkt[qpos++] = len;
        for (int i = 0; i < len; i++) qpkt[qpos++] = p[i];
        p = dot;
        if (*p == '.') p++;
    }
    qpkt[qpos++] = 0; // terminate QNAME
    qpkt[qpos++] = 0; qpkt[qpos++] = 1; // QTYPE = A (1)
    qpkt[qpos++] = 0; qpkt[qpos++] = 1; // QCLASS = IN (1)

    // Open UDP socket
    int s = sys_socket(2 /* AF_INET */, 2 /* SOCK_DGRAM */, 0);
    if (s < 0) return EAI_NONAME;

    struct my_sockaddr_in dns_srv;
    my_memset(&dns_srv, 0, sizeof(dns_srv));
    dns_srv.sin_family = 2; // AF_INET
    dns_srv.sin_port = 0x3500; // htons(53) = 0x0035 in be -> 0x3500 on le
    dns_srv.sin_addr = 0x0302000a; // 10.0.2.3 in le

    long sent = sys_sendto(s, qpkt, qpos, 0, &dns_srv, sizeof(dns_srv));
    if (sent < 0) {
        sys_close(s);
        return EAI_NONAME;
    }

    uint8_t resp[512];
    my_memset(resp, 0, sizeof(resp));
    long rcvd = sys_recvfrom(s, resp, sizeof(resp), 0, 0, 0);
    sys_close(s);

    if (rcvd < 12) return EAI_NONAME;
    uint16_t flags = (resp[2] << 8) | resp[3];
    if ((flags & 0x0f) == 3) {
        // NXDOMAIN
        return EAI_NONAME;
    }
    if ((flags & 0x8000) == 0) return EAI_NONAME; // not a response

    uint16_t qdcount = (resp[4] << 8) | resp[5];
    uint16_t ancount = (resp[6] << 8) | resp[7];
    if (ancount == 0) return EAI_NONAME;

    // Skip question section
    int offset = 12;
    for (int q = 0; q < qdcount; q++) {
        while (offset < rcvd) {
            uint8_t len = resp[offset++];
            if (len == 0) break;
            if ((len & 0xc0) == 0xc0) { offset++; break; }
            offset += len;
        }
        offset += 4; // QTYPE + QCLASS
    }

    // Parse answers
    uint32_t resolved_ip = 0;
    for (int a = 0; a < ancount && offset < rcvd; a++) {
        while (offset < rcvd) {
            uint8_t len = resp[offset++];
            if (len == 0) break;
            if ((len & 0xc0) == 0xc0) { offset++; break; }
            offset += len;
        }
        if (offset + 10 > rcvd) break;
        uint16_t rtype = (resp[offset] << 8) | resp[offset + 1];
        uint16_t rdlength = (resp[offset + 8] << 8) | resp[offset + 9];
        offset += 10;
        if (offset + rdlength > rcvd) break;
        if (rtype == 1 && rdlength == 4) {
            resolved_ip = *(uint32_t *)&resp[offset];
            break;
        }
        offset += rdlength;
    }

    if (resolved_ip == 0) return EAI_NONAME;

    my_memset(&g_sockaddr, 0, sizeof(g_sockaddr));
    g_sockaddr.sin_family = 2; // AF_INET
    g_sockaddr.sin_addr = resolved_ip;

    my_memset(&g_addrinfo, 0, sizeof(g_addrinfo));
    g_addrinfo.ai_family = 2;
    g_addrinfo.ai_socktype = 1; // SOCK_STREAM
    g_addrinfo.ai_addrlen = sizeof(g_sockaddr);
    g_addrinfo.ai_addr = &g_sockaddr;

    *res = &g_addrinfo;
    return 0;
}

__attribute__((visibility("default")))
void freeaddrinfo(void *res) {
    (void)res;
}

__attribute__((visibility("default")))
struct my_hostent *gethostbyname(const char *name) {
    struct my_addrinfo *res = 0;
    if (getaddrinfo(name, 0, 0, &res) != 0 || !res) {
        return 0;
    }
    struct my_sockaddr_in *sin = res->ai_addr;
    g_host_addr = sin->sin_addr;
    g_addr_list[0] = (char *)&g_host_addr;
    g_addr_list[1] = 0;

    my_memset(&g_hostent, 0, sizeof(g_hostent));
    size_t nl = my_strlen(name);
    if (nl >= sizeof(g_host_name)) nl = sizeof(g_host_name) - 1;
    my_memcpy(g_host_name, name, nl);
    g_host_name[nl] = 0;

    g_hostent.h_name = g_host_name;
    g_hostent.h_aliases = 0;
    g_hostent.h_addrtype = 2; // AF_INET
    g_hostent.h_length = 4;
    g_hostent.h_addr_list = g_addr_list;
    return &g_hostent;
}

static uintptr_t lookup_symbol(const char *name) {
    if (my_strcmp(name, "__libc_start_main") == 0) {
        return (uintptr_t)&__libc_start_main;
    }
    if (my_strcmp(name, "write") == 0) {
        return (uintptr_t)&write;
    }
    if (my_strcmp(name, "exit") == 0 || my_strcmp(name, "_exit") == 0) {
        return (uintptr_t)&exit;
    }
    if (my_strcmp(name, "getaddrinfo") == 0) {
        return (uintptr_t)&getaddrinfo;
    }
    if (my_strcmp(name, "freeaddrinfo") == 0) {
        return (uintptr_t)&freeaddrinfo;
    }
    if (my_strcmp(name, "gethostbyname") == 0) {
        return (uintptr_t)&gethostbyname;
    }
    for (int i = 0; i < loaded_lib_count; i++) {
        LoadedLib *lib = &loaded_libs[i];
        for (size_t s = 1; s < lib->sym_count; s++) {
            Elf64_Sym *sym = &lib->symtab[s];
            if (sym->st_shndx != 0 && sym->st_name) {
                const char *sym_name = lib->strtab + sym->st_name;
                if (my_strcmp(sym_name, name) == 0) {
                    return lib->base_addr + sym->st_value;
                }
            }
        }
    }
    return 0;
}

static void load_shared_library(const char *libname) {
    if (my_strcmp(libname, "libc.so") == 0 || my_strcmp(libname, "ld-musl-x86_64.so.1") == 0) {
        log_str("[ldso] skipping built-in libc provider: ");
        log_str(libname);
        log_str("\n");
        return;
    }
    for (int i = 0; i < loaded_lib_count; i++) {
        if (my_strcmp(loaded_libs[i].name, libname) == 0) {
            return;
        }
    }
    log_str("[ldso] loading shared library: ");
    log_str(libname);
    log_str("\n");

    char path[128];
    path[0] = '/';
    path[1] = 'l';
    path[2] = 'i';
    path[3] = 'b';
    path[4] = '/';
    size_t n = 0;
    while (libname[n] && n < 100) {
        path[5 + n] = libname[n];
        n++;
    }
    path[5 + n] = 0;

    int fd = sys_open(path, 0);
    if (fd < 0 || (uintptr_t)fd > 0xffff000000000000ULL) {
        fd = sys_open(libname, 0);
    }
    if (fd < 0 || (uintptr_t)fd > 0xffff000000000000ULL) {
        log_str("[ldso] ERROR: unable to open ");
        log_str(path);
        log_str("\n");
        sys_exit(127);
    }

    Elf64_Ehdr ehdr;
    if (sys_read(fd, &ehdr, sizeof(ehdr)) != sizeof(ehdr)) {
        log_str("[ldso] ERROR: failed reading ELF header\n");
        sys_exit(127);
    }

    Elf64_Phdr phdrs[16];
    if (ehdr.e_phnum > 16) {
        log_str("[ldso] ERROR: too many program headers\n");
        sys_exit(127);
    }
    sys_lseek(fd, ehdr.e_phoff, 0);
    sys_read(fd, phdrs, ehdr.e_phnum * sizeof(Elf64_Phdr));

    uintptr_t min_vaddr = (uintptr_t)-1;
    uintptr_t max_vaddr = 0;
    Elf64_Phdr *dynamic_phdr = NULL;
    for (int i = 0; i < ehdr.e_phnum; i++) {
        if (phdrs[i].p_type == PT_LOAD) {
            if (phdrs[i].p_vaddr < min_vaddr) min_vaddr = phdrs[i].p_vaddr;
            uintptr_t end = phdrs[i].p_vaddr + phdrs[i].p_memsz;
            if (end > max_vaddr) max_vaddr = end;
        } else if (phdrs[i].p_type == PT_DYNAMIC) {
            dynamic_phdr = &phdrs[i];
        }
    }
    min_vaddr &= ~4095ULL;
    max_vaddr = (max_vaddr + 4095ULL) & ~4095ULL;
    uintptr_t total_size = max_vaddr - min_vaddr;

    uintptr_t lib_base = (uintptr_t)sys_mmap(
        (void *)next_lib_alloc,
        total_size,
        PROT_READ | PROT_WRITE | PROT_EXEC,
        MAP_PRIVATE | MAP_ANONYMOUS | MAP_FIXED,
        -1,
        0
    );
    if ((intptr_t)lib_base < 0) {
        log_str("[ldso] ERROR: mmap failed for library\n");
        sys_exit(127);
    }
    next_lib_alloc += (total_size + 0x200000ULL - 1) & ~(0x200000ULL - 1);

    log_str("[ldso] mapped library at ");
    log_hex(lib_base);
    log_str(" size ");
    log_hex(total_size);
    log_str("\n");

    for (int i = 0; i < ehdr.e_phnum; i++) {
        if (phdrs[i].p_type == PT_LOAD) {
            void *dest = (void *)(lib_base + phdrs[i].p_vaddr);
            sys_lseek(fd, phdrs[i].p_offset, 0);
            sys_read(fd, dest, phdrs[i].p_filesz);
            if (phdrs[i].p_memsz > phdrs[i].p_filesz) {
                my_memset(
                    (void *)((uintptr_t)dest + phdrs[i].p_filesz),
                    0,
                    phdrs[i].p_memsz - phdrs[i].p_filesz
                );
            }
        }
    }
    sys_close(fd);

    if (!dynamic_phdr) {
        log_str("[ldso] ERROR: no PT_DYNAMIC in library\n");
        sys_exit(127);
    }

    Elf64_Dyn *dyn = (Elf64_Dyn *)(lib_base + dynamic_phdr->p_vaddr);
    const char *strtab = NULL;
    Elf64_Sym *symtab = NULL;
    uint32_t *hash = NULL;
    size_t sym_count = 0;

    Elf64_Rela *lib_rela = NULL;
    size_t lib_relasz = 0;

    for (int i = 0; dyn[i].d_tag != DT_NULL; i++) {
        uint64_t val = dyn[i].d_un.d_val;
        uintptr_t ptr = (val < lib_base) ? (lib_base + val) : val;
        switch (dyn[i].d_tag) {
            case DT_STRTAB: strtab = (const char *)ptr; break;
            case DT_SYMTAB: symtab = (Elf64_Sym *)ptr; break;
            case DT_HASH:   hash = (uint32_t *)ptr; break;
            case DT_RELA:   lib_rela = (Elf64_Rela *)ptr; break;
            case DT_RELASZ: lib_relasz = val; break;
        }
    }
    if (hash) {
        sym_count = hash[1];
    } else {
        sym_count = 64;
    }

    if (lib_rela && lib_relasz) {
        size_t count = lib_relasz / sizeof(Elf64_Rela);
        for (size_t r = 0; r < count; r++) {
            Elf64_Rela *rela = &lib_rela[r];
            uintptr_t *target = (uintptr_t *)(lib_base + rela->r_offset);
            uint32_t type = ELF64_R_TYPE(rela->r_info);
            uint32_t sym_idx = ELF64_R_SYM(rela->r_info);

            if (type == R_X86_64_RELATIVE) {
                *target = lib_base + rela->r_addend;
            } else if (type == R_X86_64_GLOB_DAT || type == R_X86_64_64) {
                if (sym_idx < sym_count) {
                    Elf64_Sym *sym = &symtab[sym_idx];
                    uintptr_t sym_val = 0;
                    if (sym->st_shndx != 0) {
                        sym_val = lib_base + sym->st_value;
                    } else if (sym->st_name) {
                        const char *sym_name = strtab + sym->st_name;
                        sym_val = lookup_symbol(sym_name);
                    }
                    if (sym_val) {
                        log_str("[ldso] library GOT relocated (GLOB_DAT): ");
                        log_str(strtab + sym->st_name);
                        log_str(" -> ");
                        log_hex(sym_val);
                        log_str("\n");
                        *target = sym_val + (type == R_X86_64_64 ? rela->r_addend : 0);
                    }
                }
            }
        }
    }

    LoadedLib *lib = &loaded_libs[loaded_lib_count++];
    n = 0;
    while (libname[n] && n < 63) {
        lib->name[n] = libname[n];
        n++;
    }
    lib->name[n] = 0;
    lib->base_addr = lib_base;
    lib->symtab = symtab;
    lib->strtab = strtab;
    lib->sym_count = sym_count;
}

__attribute__((visibility("hidden")))
uintptr_t _dl_entry(uintptr_t *sp) {
    uintptr_t argc = *sp++;
    char **argv = (char **)sp;
    sp += argc + 1; // skip argv and NULL
    char **envp = (char **)sp;
    while (*sp) sp++; // skip envp
    sp++; // skip NULL terminating envp

    uintptr_t at_phdr = 0;
    uintptr_t at_phent = 56;
    uintptr_t at_phnum = 0;
    uintptr_t at_entry = 0;
    uintptr_t at_base = 0;

    while (*sp != AT_NULL) {
        uintptr_t key = *sp++;
        uintptr_t val = *sp++;
        switch (key) {
            case AT_PHDR:  at_phdr = val; break;
            case AT_PHENT: at_phent = val; break;
            case AT_PHNUM: at_phnum = val; break;
            case AT_ENTRY: at_entry = val; break;
            case AT_BASE:  at_base = val; break;
        }
    }

    log_str("[ldso] dynamic linker active\n");
    log_str("[ldso] AT_PHDR=");
    log_hex(at_phdr);
    log_str(" AT_ENTRY=");
    log_hex(at_entry);
    log_str("\n");

    Elf64_Phdr *phdrs = (Elf64_Phdr *)at_phdr;
    uintptr_t main_base = 0;
    Elf64_Phdr *main_dynamic_phdr = NULL;

    for (uintptr_t i = 0; i < at_phnum; i++) {
        if (phdrs[i].p_type == PT_PHDR) {
            main_base = at_phdr - phdrs[i].p_vaddr;
        } else if (phdrs[i].p_type == PT_DYNAMIC) {
            main_dynamic_phdr = &phdrs[i];
        }
    }
    if (main_base == 0 && at_phnum > 0 && phdrs[0].p_type == PT_LOAD) {
        main_base = (at_phdr - phdrs[0].p_offset) - phdrs[0].p_vaddr;
    }

    log_str("[ldso] main executable base=");
    log_hex(main_base);
    log_str("\n");

    if (!main_dynamic_phdr) {
        log_str("[ldso] static/non-dynamic binary: jumping directly to AT_ENTRY\n");
        return at_entry;
    }

    Elf64_Dyn *dyn = (Elf64_Dyn *)(main_base + main_dynamic_phdr->p_vaddr);
    const char *main_strtab = NULL;
    Elf64_Sym *main_symtab = NULL;
    Elf64_Rela *rela_dyn = NULL;
    size_t rela_dyn_size = 0;
    Elf64_Rela *rela_plt = NULL;
    size_t rela_plt_size = 0;

    for (int i = 0; dyn[i].d_tag != DT_NULL; i++) {
        uint64_t val = dyn[i].d_un.d_val;
        uintptr_t ptr = (val < main_base) ? (main_base + val) : val;
        switch (dyn[i].d_tag) {
            case DT_STRTAB:   main_strtab = (const char *)ptr; break;
            case DT_SYMTAB:   main_symtab = (Elf64_Sym *)ptr; break;
            case DT_RELA:     rela_dyn = (Elf64_Rela *)ptr; break;
            case DT_RELASZ:   rela_dyn_size = val; break;
            case DT_JMPREL:   rela_plt = (Elf64_Rela *)ptr; break;
            case DT_PLTRELSZ: rela_plt_size = val; break;
        }
    }

    // Pass 1: Load all DT_NEEDED libraries
    for (int i = 0; dyn[i].d_tag != DT_NULL; i++) {
        if (dyn[i].d_tag == DT_NEEDED) {
            const char *libname = main_strtab + dyn[i].d_un.d_val;
            load_shared_library(libname);
        }
    }

    // Pass 2: Apply relocations in .rela.dyn
    if (rela_dyn && rela_dyn_size) {
        size_t count = rela_dyn_size / sizeof(Elf64_Rela);
        for (size_t r = 0; r < count; r++) {
            Elf64_Rela *rela = &rela_dyn[r];
            uintptr_t *target = (uintptr_t *)(main_base + rela->r_offset);
            uint32_t type = ELF64_R_TYPE(rela->r_info);
            uint32_t sym_idx = ELF64_R_SYM(rela->r_info);

            if (type == R_X86_64_RELATIVE) {
                *target = main_base + rela->r_addend;
            } else if (type == R_X86_64_GLOB_DAT || type == R_X86_64_64) {
                const char *sym_name = main_strtab + main_symtab[sym_idx].st_name;
                uintptr_t sym_val = lookup_symbol(sym_name);
                if (sym_val) {
                    log_str("[ldso] resolved data symbol (GLOB_DAT): ");
                    log_str(sym_name);
                    log_str(" -> ");
                    log_hex(sym_val);
                    log_str(" (GOT slot ");
                    log_hex((uintptr_t)target);
                    log_str(")\n");
                    *target = sym_val + (type == R_X86_64_64 ? rela->r_addend : 0);
                } else {
                    log_str("[ldso] ERROR: unresolved data symbol: ");
                    log_str(sym_name);
                    log_str("\n");
                    sys_exit(127);
                }
            } else if (type == R_X86_64_COPY) {
                const char *sym_name = main_strtab + main_symtab[sym_idx].st_name;
                uintptr_t sym_val = lookup_symbol(sym_name);
                size_t sz = main_symtab[sym_idx].st_size;
                if (sym_val) {
                    log_str("[ldso] resolved data copy relocation (COPY): ");
                    log_str(sym_name);
                    log_str(" size ");
                    log_hex(sz);
                    log_str(" (target ");
                    log_hex((uintptr_t)target);
                    log_str(")\n");
                    my_memcpy((void *)target, (const void *)sym_val, sz);
                } else {
                    log_str("[ldso] ERROR: unresolved copy symbol: ");
                    log_str(sym_name);
                    log_str("\n");
                    sys_exit(127);
                }
            }
        }
    }

    // Pass 3: Apply relocations in .rela.plt
    if (rela_plt && rela_plt_size) {
        size_t count = rela_plt_size / sizeof(Elf64_Rela);
        for (size_t r = 0; r < count; r++) {
            Elf64_Rela *rela = &rela_plt[r];
            uintptr_t *target = (uintptr_t *)(main_base + rela->r_offset);
            uint32_t type = ELF64_R_TYPE(rela->r_info);
            uint32_t sym_idx = ELF64_R_SYM(rela->r_info);

            if (type == R_X86_64_JUMP_SLOT || type == R_X86_64_GLOB_DAT) {
                const char *sym_name = main_strtab + main_symtab[sym_idx].st_name;
                uintptr_t sym_val = lookup_symbol(sym_name);
                if (sym_val) {
                    log_str("[ldso] resolved symbol: ");
                    log_str(sym_name);
                    log_str(" -> ");
                    log_hex(sym_val);
                    log_str(" (GOT slot ");
                    log_hex((uintptr_t)target);
                    log_str(")\n");
                    *target = sym_val;
                } else {
                    log_str("[ldso] ERROR: unresolved symbol: ");
                    log_str(sym_name);
                    log_str("\n");
                    sys_exit(127);
                }
            }
        }
    }

    log_str("[ldso] relocations complete: jumping to AT_ENTRY ");
    log_hex(at_entry);
    log_str("\n");
    return at_entry;
}

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "mov %rsp, %rdi\n"
        "mov %rsp, %r12\n"
        "and $-16, %rsp\n"
        "lea _dl_entry(%rip), %rax\n"
        "call *%rax\n"
        "mov %r12, %rsp\n"
        "mov %rax, %r11\n"
        "xor %rdx, %rdx\n"
        "jmp *%r11\n"
    );
}
