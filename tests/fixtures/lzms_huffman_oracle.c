#include <stdio.h>
#include <stdint.h>
extern void make_canonical_huffman_code(unsigned, unsigned, const uint32_t*, uint8_t*, uint32_t*);
int main(void) {
    uint32_t freqs[20], words[20]; uint8_t lens[20];
    freqs[0]=1; freqs[1]=1; for(int i=2;i<20;i++) freqs[i]=freqs[i-1]+freqs[i-2];
    make_canonical_huffman_code(20,15,freqs,lens,words);
    for(int i=0;i<20;i++) printf("%u%s",lens[i],i==19?"\n":", ");
}
