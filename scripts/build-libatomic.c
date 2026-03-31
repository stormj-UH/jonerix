/* Minimal libatomic: only __atomic_compare_exchange needed by Node 24 V8 */
#include <stdint.h>

int __atomic_compare_exchange(unsigned long size, void *ptr, void *expected,
                               void *desired, int success_order, int failure_order) {
    switch (size) {
    case 1: return __atomic_compare_exchange_n((_Atomic uint8_t *)ptr, (uint8_t *)expected, *(uint8_t *)desired, 0, success_order, failure_order);
    case 2: return __atomic_compare_exchange_n((_Atomic uint16_t *)ptr, (uint16_t *)expected, *(uint16_t *)desired, 0, success_order, failure_order);
    case 4: return __atomic_compare_exchange_n((_Atomic uint32_t *)ptr, (uint32_t *)expected, *(uint32_t *)desired, 0, success_order, failure_order);
    case 8: return __atomic_compare_exchange_n((_Atomic uint64_t *)ptr, (uint64_t *)expected, *(uint64_t *)desired, 0, success_order, failure_order);
    default: return 0;
    }
}
