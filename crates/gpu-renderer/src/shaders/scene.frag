precision highp float;

const float PI = 3.14159265;
const float TWO_PI = 6.2831853;

uniform sampler2D u_texture;
uniform float u_opacity;
uniform float u_time;

// Water wave: direction, scale, speed, strength.
uniform vec4 u_water_wave;
uniform sampler2D u_water_wave_mask;
uniform sampler2D u_water_wave_normal;
uniform float u_has_water_wave_mask;
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

varying vec2 v_tex_coord;

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
            texture2D(u_foliage_mask0, v_tex_coord).r,
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
            texture2D(u_foliage_mask1, v_tex_coord).r,
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
        texture2D(u_iris_mask, v_tex_coord).r,
        u_has_iris_mask
    );
    return uv + motion * u_iris_scale * 0.001 * mask;
}

vec2 applyWaterWave(vec2 uv) {
    if (u_water_wave.w <= 0.0) {
        return uv;
    }

    float mask = mix(
        1.0,
        texture2D(u_water_wave_mask, v_tex_coord).r,
        u_has_water_wave_mask
    );
    vec2 direction = directionFromAngle(u_water_wave.x);
    float scale = max(u_water_wave.y, 0.01);
    float frequency = TWO_PI / scale;
    float phase = dot(uv, direction) * frequency + u_time * u_water_wave.z;
    vec2 proceduralOffset = direction * sin(phase);

    vec2 normalUv = v_tex_coord / scale
                  + direction * u_time * u_water_wave.z * 0.02;
    vec2 normalOffset = texture2D(u_water_wave_normal, normalUv).rg * 2.0 - 1.0;
    vec2 offset = mix(proceduralOffset, normalOffset, u_has_water_wave_normal);
    return uv + offset * u_water_wave.w * 0.01 * mask;
}

vec2 applyWaterFlow(vec2 uv) {
    if (u_water_flow.z <= 0.0) {
        return uv;
    }

    float mask = mix(
        1.0,
        texture2D(u_water_flow_mask, v_tex_coord).r,
        u_has_water_flow_mask
    );
    float phase = u_time * u_water_flow.y;
    float phaseScale = max(u_water_flow.x, 0.01);
    float frequency = TWO_PI * phaseScale;
    vec2 proceduralOffset = vec2(
        sin(uv.y * frequency + phase),
        cos(uv.x * frequency - phase)
    );

    vec2 phaseUv = v_tex_coord * phaseScale
                 + vec2(phase * 0.02, -phase * 0.015);
    vec2 phaseOffset = texture2D(u_water_flow_phase, phaseUv).rg * 2.0 - 1.0;
    vec2 offset = mix(proceduralOffset, phaseOffset, u_has_water_flow_phase);
    return uv + offset * u_water_flow.z * 0.003 * mask;
}

vec4 applyShine(vec4 color) {
    if (u_shine.z <= 0.0) {
        return color;
    }

    vec2 direction = directionFromAngle(u_shine.x);
    float position = fract(
        dot(v_tex_coord - 0.5, direction) - u_time * u_shine.y
    );
    float width = clamp(u_shine.w * 0.2, 0.005, 0.45);
    float ray = 1.0 - smoothstep(0.0, width, abs(position - 0.5));
    float mask = mix(
        1.0,
        texture2D(u_shine_mask, v_tex_coord).r,
        u_has_shine_mask
    );
    color.rgb += u_shine_color * ray * mask * u_shine.z * color.a;
    return color;
}

void main() {
    vec2 uv = applyFoliageSway(v_tex_coord);
    uv = applyIrisMovement(uv);
    uv = applyWaterWave(uv);
    uv = applyWaterFlow(uv);

    vec4 color = texture2D(u_texture, uv);
    color = applyShine(color);
    gl_FragColor = vec4(color.rgb, color.a * u_opacity);
}
