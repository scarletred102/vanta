#ifndef VANTA_H
#define VANTA_H

#include <stdint.h>
#include <stddef.h>

typedef struct {
    uint32_t abi_version;
    uint32_t struct_size;
    uint64_t features;
} vanta_abi_info_t;

typedef struct {
    uint64_t size;
    uint64_t mode;
} vanta_stat_t;

typedef struct {
    uint32_t read_fd;
    uint32_t write_fd;
} vanta_pipe_t;

typedef struct {
    uint64_t fd;
} vanta_stream_t;

typedef struct {
    uint64_t fd;
    uint32_t buffer_pos;
    uint32_t buffer_len;
    uint8_t buffer[256];
    char name[257];
} vanta_dir_t;

typedef struct {
    uint64_t fd;
    uint32_t mode;
    uint32_t buffer_pos;
    uint32_t buffer_len;
    uint8_t buffer[256];
} vanta_file_t;

#define VANTA_OPEN_READ 0x10
#define VANTA_OPEN_WRITE 0x11
#define VANTA_OPEN_CREATE 0x13
#define VANTA_OPEN_TRUNCATE 0x15
#define VANTA_OPEN_APPEND 0x19

int32_t *vanta_errno_location(void);
int64_t vanta_write(uint64_t fd, const uint8_t *buffer, size_t length);
int64_t vanta_read(uint64_t fd, uint8_t *buffer, size_t length);
int64_t vanta_open(const uint8_t *path, size_t length, uint64_t flags);
int64_t vanta_close(uint64_t fd);
int64_t vanta_spawn(const uint8_t *path, size_t length);
int64_t vanta_waitpid(uint64_t pid);
int64_t vanta_get_abi_info(vanta_abi_info_t *info);
int64_t vanta_dup(uint64_t fd);
int64_t vanta_pipe(vanta_pipe_t *pipe);
int64_t vanta_fstat(uint64_t fd, vanta_stat_t *stat);
int64_t vanta_getdents(uint64_t fd, uint8_t *buffer, size_t length);
int64_t vanta_dir_open(const uint8_t *path, size_t length, vanta_dir_t *directory);
int64_t vanta_dir_read(vanta_dir_t *directory, char *name, size_t length);
int64_t vanta_dir_close(vanta_dir_t *directory);
int64_t vanta_mkdir(const uint8_t *path, size_t length);
int64_t vanta_unlink(const uint8_t *path, size_t length);
int64_t vanta_rename(const uint8_t *old_path, size_t old_length,
                    const uint8_t *new_path, size_t new_length);
int64_t vanta_getpid(void);
int64_t vanta_getppid(void);
int64_t vanta_yield(void);
int64_t vanta_kill(uint64_t pid, uint64_t signal);
int64_t vanta_sigaction(uint64_t signal, uint64_t handler, uint64_t flags);
int64_t vanta_stream_open(const uint8_t *path, size_t length, uint64_t flags,
                          vanta_stream_t *stream);
int64_t vanta_stream_read(vanta_stream_t *stream, uint8_t *buffer, size_t length);
int64_t vanta_stream_write(vanta_stream_t *stream, const uint8_t *buffer,
                           size_t length);
int64_t vanta_stream_close(vanta_stream_t *stream);
int64_t vanta_stream_flush(vanta_stream_t *stream);
int64_t vanta_file_open(const uint8_t *path, size_t length, uint64_t flags,
                        vanta_file_t *file);
int64_t vanta_file_flush(vanta_file_t *file);
int64_t vanta_file_write(vanta_file_t *file, const uint8_t *buffer, size_t length);
int64_t vanta_file_getc(vanta_file_t *file);
int64_t vanta_file_putc(vanta_file_t *file, uint8_t byte);
int64_t vanta_file_close(vanta_file_t *file);
void *vanta_malloc(size_t size);
void vanta_free(void *pointer);
void vanta_exit(int32_t status);

#endif
