#define _GNU_SOURCE
#include <dlfcn.h>
#include <fcntl.h>
#include <stdlib.h>
#include <sys/time.h>
#include <time.h>
#include <unistd.h>

/* Shift only this child process's wall clock. Monotonic timers stay real. */
int clock_gettime(clockid_t clock_id, struct timespec *value) {
    int (*original)(clockid_t, struct timespec *) = dlsym(RTLD_NEXT, "clock_gettime");
    int result = original(clock_id, value);
    if (result == 0 && clock_id == CLOCK_REALTIME) {
        const char *path = getenv("THIRDSHIFT_REVIEW_CLOCK_OFFSET");
        if (path) {
            int fd = open(path, O_RDONLY | O_CLOEXEC);
            if (fd >= 0) {
                char buffer[64] = {0};
                if (read(fd, buffer, sizeof(buffer) - 1) > 0) {
                    value->tv_sec += strtoll(buffer, NULL, 10);
                }
                close(fd);
            }
        }
    }
    return result;
}

time_t time(time_t *destination) {
    struct timespec value;
    if (clock_gettime(CLOCK_REALTIME, &value) != 0) return (time_t)-1;
    if (destination) *destination = value.tv_sec;
    return value.tv_sec;
}

int gettimeofday(struct timeval *value, void *timezone) {
    (void)timezone;
    struct timespec current;
    int result = clock_gettime(CLOCK_REALTIME, &current);
    if (result == 0 && value) {
        value->tv_sec = current.tv_sec;
        value->tv_usec = current.tv_nsec / 1000;
    }
    return result;
}
