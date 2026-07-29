#ifndef VANTA_H
#define VANTA_H

#include <stdint.h>
#include <stddef.h>

int32_t *vanta_errno_location(void);
int64_t vanta_write(uint64_t fd, const uint8_t *buffer, size_t length);
int64_t vanta_read(uint64_t fd, uint8_t *buffer, size_t length);
int64_t vanta_open(const uint8_t *path, size_t length, uint64_t flags);
int64_t vanta_close(uint64_t fd);
int64_t vanta_spawn(const uint8_t *path, size_t length);
int64_t vanta_waitpid(uint64_t pid);
void *vanta_malloc(size_t size);
void vanta_free(void *pointer);
void vanta_exit(int32_t status);

#endif
