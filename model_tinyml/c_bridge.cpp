#include <stdint.h>
#include <stddef.h>

#include "edge-impulse-sdk/classifier/ei_run_classifier.h"

extern "C" {

int edge_impulse_predict(
    const float* raw_features,
    size_t features_count,
    int* out_class,
    float* out_confidence
) {
    signal_t signal;
    int err = numpy::signal_from_buffer(raw_features, features_count, &signal);
    if (err != 0) {
        return err;
    }

    ei_impulse_result_t result = { 0 };
    EI_IMPULSE_ERROR res = run_classifier(&signal, &result, false);
    if (res != EI_IMPULSE_OK) {
        return -1;
    }

    float max_val = 0.0f;
    int max_idx = 0;

    // 2 kelas (Arabika & Robusta)
    for (size_t ix = 0; ix < 2; ix++) {
        if (result.classification[ix].value > max_val) {
            max_val = result.classification[ix].value;
            max_idx = (int)ix;
        }
    }

    *out_class = max_idx;
    *out_confidence = max_val;
    return 0;
}

}