precision highp float;

const float PI = 3.14159265;
const float TWO_PI = 6.2831853;

uniform sampler2D u_texture;
uniform float u_opacity;
uniform float u_time;
// Maps a displacement in logical layer UV space into the potentially padded
// storage UV space of the main texture.
uniform vec2 u_texture_uv_scale;

// Water wave: direction, scale, speed, strength.
uniform vec4 u_water_wave0;
uniform vec4 u_water_wave1;
uniform vec4 u_water_wave2;
uniform sampler2D u_water_wave_mask0;
uniform sampler2D u_water_wave_mask1;
uniform sampler2D u_water_wave_mask2;
uniform sampler2D u_water_wave_normal;
uniform float u_has_water_wave_mask0;
uniform float u_has_water_wave_mask1;
uniform float u_has_water_wave_mask2;
uniform float u_has_water_wave_normal;

// Water flow: phase scale, speed, strength.
uniform vec3 u_water_flow;
uniform sampler2D u_water_flow_mask;
uniform sampler2D u_water_flow_phase;
uniform float u_has_water_flow_mask;
uniform float u_has_water_flow_phase;

// Iris movement: speed, roughness, noise amount, phase.
uniform vec4 u_iris;
uniform vec2 u_iris_scale;
uniform sampler2D u_iris_mask;
uniform float u_has_iris_mask;

// Foliage sway: direction, scale, speed, strength.
// Extra parameters: phase, power, aspect ratio.
uniform vec4 u_foliage0;
uniform vec3 u_foliage0_extra;
uniform sampler2D u_foliage_mask0;
uniform float u_has_foliage_mask0;
uniform vec4 u_foliage1;
uniform vec3 u_foliage1_extra;
uniform sampler2D u_foliage_mask1;
uniform float u_has_foliage_mask1;

// Shine: direction, speed, intensity, length.
uniform vec4 u_shine;
uniform vec3 u_shine_color;
uniform sampler2D u_shine_mask;
uniform float u_has_shine_mask;

// Direction-map shake: speed, strength, lower bound, upper bound.
uniform vec4 u_shake0;
uniform vec4 u_shake1;
uniform vec4 u_shake2;
uniform vec2 u_shake_friction0;
uniform vec2 u_shake_friction1;
uniform vec2 u_shake_friction2;
uniform sampler2D u_shake_map0;
uniform sampler2D u_shake_map1;
uniform sampler2D u_shake_map2;
uniform float u_has_shake_map0;
uniform float u_has_shake_map1;
uniform float u_has_shake_map2;

// Color pulse: speed, phase, amount, power.
uniform vec4 u_pulse0;
uniform vec4 u_pulse1;
uniform vec4 u_pulse2;
uniform vec2 u_pulse_bounds0;
uniform vec2 u_pulse_bounds1;
uniform vec2 u_pulse_bounds2;
uniform vec3 u_pulse_low0;
uniform vec3 u_pulse_low1;
uniform vec3 u_pulse_low2;
uniform vec3 u_pulse_high0;
uniform vec3 u_pulse_high1;
uniform vec3 u_pulse_high2;
uniform sampler2D u_pulse_mask0;
uniform sampler2D u_pulse_mask1;
uniform sampler2D u_pulse_mask2;
uniform float u_has_pulse_mask0;
uniform float u_has_pulse_mask1;
uniform float u_has_pulse_mask2;

// Texture-space spin: speed, center x/y.
uniform vec3 u_spin;
uniform float u_spin_aspect;
uniform float u_has_spin;

varying vec2 v_tex_coord;
varying vec2 v_effect_coord;

vec2 directionFromAngle(float angle) {
    return vec2(cos(angle), sin(angle));
}

float signedPower(float value, float exponent) {
    return sign(value) * pow(abs(value), exponent);
}

vec2 calculateSwayOffset(vec2 coord, vec4 sway, vec3 extra, float mask) {
    vec2 direction = directionFromAngle(sway.x);
    vec2 perpendicular = vec2(-direction.y, direction.x);
    float spatialPhase = dot(coord, direction) * 10.0
                       + dot(coord, perpendicular) * 5.0;
    float phase = spatialPhase * extra.x + u_time * sway.z;

    float primaryWave = sin(phase) - 0.16161616 * sin(phase * 0.5 + 0.4);
    float crossWave = sin(phase * -0.5 + 0.4)
                    + 0.041666666 * sin(phase * 0.25);
    float power = max(extra.y, 0.01);
    primaryWave = signedPower(primaryWave, power);
    crossWave = signedPower(crossWave, power);

    float amplitude = sway.w * sway.w * 0.005 * mask;
    float aspectRatio = max(extra.z, 0.01);
    return vec2(primaryWave, crossWave * aspectRatio) * amplitude;
}

vec2 applyFoliageSway(vec2 uv) {
    if (u_foliage0.w > 0.0) {
        float mask = mix(
            1.0,
            texture2D(u_foliage_mask0, v_effect_coord).r,
            u_has_foliage_mask0
        );
        uv += calculateSwayOffset(
            uv * max(u_foliage0.y, 0.001),
            u_foliage0,
            u_foliage0_extra,
            mask
        );
    }

    if (u_foliage1.w > 0.0) {
        float mask = mix(
            1.0,
            texture2D(u_foliage_mask1, v_effect_coord).r,
            u_has_foliage_mask1
        );
        uv += calculateSwayOffset(
            uv * max(u_foliage1.y, 0.001),
            u_foliage1,
            u_foliage1_extra,
            mask
        );
    }

    return uv;
}

vec2 applyIrisMovement(vec2 uv) {
    if (u_iris.x <= 0.0) {
        return uv;
    }

    float time = u_time * u_iris.x + u_iris.w;
    float previousStep = floor(time);
    float nextStep = previousStep + 1.0;
    vec2 start = vec2(
        sin(1.9 * previousStep) + sin(2.5 * previousStep + 1.0),
        sin(1.9 * previousStep) + sin(2.5 * previousStep + 2.0)
    );
    vec2 end = vec2(
        sin(1.9 * nextStep) + sin(2.5 * nextStep + 1.0),
        sin(1.9 * nextStep) + sin(2.5 * nextStep + 2.0)
    );
    float transition = cos(fract(time) * PI) * -0.5 + 0.5;
    float blend = smoothstep(1.0 - u_iris.y, 1.0, transition);
    vec2 motion = mix(start, end, blend);
    motion += vec2(sin(time), cos(time)) * u_iris.z;

    float mask = mix(
        1.0,
        texture2D(u_iris_mask, v_effect_coord).r,
        u_has_iris_mask
    );
    return uv + motion * u_iris_scale * 0.001 * mask;
}

vec2 applyWaterWaveEffect(
    vec2 uv,
    vec4 wave,
    sampler2D waveMask,
    float hasMask
) {
    if (wave.w <= 0.0) {
        return uv;
    }

    float mask = mix(
        1.0,
        texture2D(waveMask, v_effect_coord).r,
        hasMask
    );
    vec2 direction = vec2(-sin(wave.x), cos(wave.x));
    vec2 lateral = vec2(direction.y, -direction.x);
    float distance = u_time * wave.z
                   + dot(uv - 0.5, direction) * max(wave.y, 0.01);
    return uv + sin(distance) * lateral * wave.w * wave.w * mask;
}

vec2 applyWaterWaves(vec2 uv) {
    uv = applyWaterWaveEffect(
        uv, u_water_wave0, u_water_wave_mask0, u_has_water_wave_mask0
    );
    uv = applyWaterWaveEffect(
        uv, u_water_wave1, u_water_wave_mask1, u_has_water_wave_mask1
    );
    uv = applyWaterWaveEffect(
        uv, u_water_wave2, u_water_wave_mask2, u_has_water_wave_mask2
    );
    return uv;
}

vec2 applyShakeEffect(
    vec2 uv,
    vec4 shake,
    vec2 friction,
    sampler2D directionMap,
    float hasMap
) {
    if (shake.y <= 0.0 || hasMap <= 0.0) {
        return uv;
    }
    float oscillation = sin(u_time * shake.x) * 0.5 + 0.5;
    float lower = clamp(shake.z, 0.0, 1.0);
    float upper = max(shake.w, lower + 0.0001);
    float amount = smoothstep(lower, upper, oscillation);
    amount = mix(
        1.0 - pow(1.0 - amount, max(friction.x, 0.01)),
        pow(amount, max(friction.y, 0.01)),
        step(0.0, cos(u_time * shake.x))
    );
    vec2 flow = (texture2D(directionMap, v_effect_coord).rg - vec2(0.498)) * 2.0;
    return uv + amount * shake.y * shake.y * flow;
}

vec2 applyShakes(vec2 uv) {
    uv = applyShakeEffect(
        uv, u_shake0, u_shake_friction0, u_shake_map0, u_has_shake_map0
    );
    uv = applyShakeEffect(
        uv, u_shake1, u_shake_friction1, u_shake_map1, u_has_shake_map1
    );
    uv = applyShakeEffect(
        uv, u_shake2, u_shake_friction2, u_shake_map2, u_has_shake_map2
    );
    return uv;
}

vec2 applySpin(vec2 uv) {
    if (u_has_spin <= 0.0) {
        return uv;
    }
    vec2 centered = uv - u_spin.yz;
    centered.x *= max(u_spin_aspect, 0.0001);
    float angle = u_spin.x * u_time;
    float sine = sin(angle);
    float cosine = cos(angle);
    centered = mat2(cosine, sine, -sine, cosine) * centered;
    centered.x /= max(u_spin_aspect, 0.0001);
    return centered + u_spin.yz;
}

vec2 applyWaterFlow(vec2 uv) {
    if (u_water_flow.z <= 0.0) {
        return uv;
    }

    float phaseScale = max(u_water_flow.x, 0.01);
    vec2 proceduralFlow = vec2(
        sin(uv.y * TWO_PI * phaseScale),
        cos(uv.x * TWO_PI * phaseScale)
    );
    vec2 mappedFlow = (texture2D(u_water_flow_mask, v_effect_coord).rg - vec2(0.498)) * 2.0;
    vec2 flow = mix(proceduralFlow, mappedFlow, u_has_water_flow_mask);
    float authoredPhase = texture2D(
        u_water_flow_phase, v_effect_coord * phaseScale
    ).r;
    float phase = mix(0.0, authoredPhase, u_has_water_flow_phase);
    float cycle = sin((u_time * u_water_flow.y + phase) * TWO_PI);
    return uv + flow * cycle * u_water_flow.z * 0.05;
}

vec4 applyShine(vec4 color) {
    if (u_shine.z <= 0.0) {
        return color;
    }

    vec2 direction = directionFromAngle(u_shine.x);
    float position = fract(
        dot(v_effect_coord - 0.5, direction) - u_time * u_shine.y
    );
    float width = clamp(u_shine.w * 0.2, 0.005, 0.45);
    float ray = 1.0 - smoothstep(0.0, width, abs(position - 0.5));
    float mask = mix(
        1.0,
        texture2D(u_shine_mask, v_effect_coord).r,
        u_has_shine_mask
    );
    color.rgb += u_shine_color * ray * mask * u_shine.z * color.a;
    return color;
}

vec4 applyPulseEffect(
    vec4 color,
    vec4 pulse,
    vec2 bounds,
    vec3 tintLow,
    vec3 tintHigh,
    sampler2D pulseMask,
    float hasMask
) {
    if (pulse.z <= 0.0) {
        return color;
    }
    float value = sin(u_time * pulse.x + pulse.y) * 0.5 + 0.5;
    value = smoothstep(bounds.x, max(bounds.y, bounds.x + 0.0001), value);
    value = pow(max(value * pulse.z, 0.0), max(pulse.w, 0.01));
    vec3 tinted = color.rgb * mix(tintLow, tintHigh, value);
    float mask = mix(1.0, texture2D(pulseMask, v_effect_coord).r, hasMask);
    color.rgb = mix(color.rgb, tinted, mask);
    return color;
}

vec4 applyPulses(vec4 color) {
    color = applyPulseEffect(
        color, u_pulse0, u_pulse_bounds0, u_pulse_low0, u_pulse_high0,
        u_pulse_mask0, u_has_pulse_mask0
    );
    color = applyPulseEffect(
        color, u_pulse1, u_pulse_bounds1, u_pulse_low1, u_pulse_high1,
        u_pulse_mask1, u_has_pulse_mask1
    );
    color = applyPulseEffect(
        color, u_pulse2, u_pulse_bounds2, u_pulse_low2, u_pulse_high2,
        u_pulse_mask2, u_has_pulse_mask2
    );
    return color;
}

void main() {
    // Effects and their masks are authored against the logical image bounds,
    // not the padded dimensions used by the main TEX allocation.
    vec2 effectUv = applySpin(v_effect_coord);
    effectUv = applyFoliageSway(effectUv);
    effectUv = applyIrisMovement(effectUv);
    effectUv = applyWaterWaves(effectUv);
    effectUv = applyShakes(effectUv);
    effectUv = applyWaterFlow(effectUv);
    vec2 uv = v_tex_coord + (effectUv - v_effect_coord) * u_texture_uv_scale;

    vec4 color = texture2D(u_texture, uv);
    color = applyPulses(color);
    color = applyShine(color);
    gl_FragColor = vec4(color.rgb, color.a * u_opacity);
}
