//! Shared helpers for the renderer pass modules: bind-group-layout builder
//! and named (blend, cull, depth) recipes used across the pipelines.

use super::super::pipelines::{self, DepthConfig, PipelineDesc};

/// Fluent builder for `wgpu::BindGroupLayout` descriptors.
pub(super) struct BindGroupLayoutBuilder<'a> {
    label: &'a str,
    entries: Vec<wgpu::BindGroupLayoutEntry>,
}

impl<'a> BindGroupLayoutBuilder<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            entries: Vec::new(),
        }
    }

    pub fn entry(mut self, entry: wgpu::BindGroupLayoutEntry) -> Self {
        self.entries.push(entry);
        self
    }

    pub fn texture(
        self,
        binding: u32,
        visibility: wgpu::ShaderStages,
        view_dimension: wgpu::TextureViewDimension,
        sample_type: wgpu::TextureSampleType,
    ) -> Self {
        self.entry(wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                view_dimension,
                sample_type,
            },
            count: None,
        })
    }

    pub fn texture_2d_filterable(self, binding: u32, visibility: wgpu::ShaderStages) -> Self {
        self.texture(
            binding,
            visibility,
            wgpu::TextureViewDimension::D2,
            wgpu::TextureSampleType::Float { filterable: true },
        )
    }

    pub fn texture_2d_array_filterable(self, binding: u32, visibility: wgpu::ShaderStages) -> Self {
        self.texture(
            binding,
            visibility,
            wgpu::TextureViewDimension::D2Array,
            wgpu::TextureSampleType::Float { filterable: true },
        )
    }

    pub fn depth_texture(self, binding: u32, visibility: wgpu::ShaderStages) -> Self {
        self.texture(
            binding,
            visibility,
            wgpu::TextureViewDimension::D2,
            wgpu::TextureSampleType::Depth,
        )
    }

    pub fn sampler(
        self,
        binding: u32,
        visibility: wgpu::ShaderStages,
        ty: wgpu::SamplerBindingType,
    ) -> Self {
        self.entry(wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Sampler(ty),
            count: None,
        })
    }

    pub fn sampler_filtering(self, binding: u32, visibility: wgpu::ShaderStages) -> Self {
        self.sampler(binding, visibility, wgpu::SamplerBindingType::Filtering)
    }

    pub fn sampler_comparison(self, binding: u32, visibility: wgpu::ShaderStages) -> Self {
        self.sampler(binding, visibility, wgpu::SamplerBindingType::Comparison)
    }

    pub fn buffer(
        self,
        binding: u32,
        visibility: wgpu::ShaderStages,
        ty: wgpu::BufferBindingType,
    ) -> Self {
        self.entry(wgpu::BindGroupLayoutEntry {
            binding,
            visibility,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        })
    }

    pub fn uniform_buffer(self, binding: u32, visibility: wgpu::ShaderStages) -> Self {
        self.buffer(binding, visibility, wgpu::BufferBindingType::Uniform)
    }

    pub fn build(self, device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(self.label),
            entries: &self.entries,
        })
    }
}

/// Reusable (blend, cull, depth) triples shared across pipelines.
#[derive(Clone, Copy)]
pub(super) struct PipelineRecipe {
    pub blend: Option<wgpu::BlendState>,
    pub cull_mode: Option<wgpu::Face>,
    pub depth: DepthConfig,
}

impl PipelineRecipe {
    /// Opaque terrain. `cull_mode=None` because winding flips with camera angle.
    pub const TERRAIN_OPAQUE: Self = Self {
        blend: Some(wgpu::BlendState::REPLACE),
        cull_mode: None,
        depth: DepthConfig::WriteDefault,
    };

    /// Sky: fullscreen triangle drawn at z=1, replace blend, depth read-only.
    pub const SKY: Self = Self {
        blend: Some(wgpu::BlendState::REPLACE),
        cull_mode: None,
        depth: DepthConfig::ReadOnly,
    };

    /// Models (structures, features): alpha blended, back-face culled, depth write.
    pub const MODEL: Self = Self {
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        cull_mode: Some(wgpu::Face::Back),
        depth: DepthConfig::WriteDefault,
    };

    /// Alpha-blended overlay drawn on top of the depth buffer without writing
    /// to it. Shared by the particle and grid pipelines.
    pub const OVERLAY_ALPHA: Self = Self {
        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
        cull_mode: None,
        depth: DepthConfig::ReadOnly,
    };
}

/// Shared depth bias used by the border, heatmap, and water overlays to
/// avoid z-fighting against terrain on far/sloped pixels.
pub(super) const OVERLAY_DEPTH_BIAS: wgpu::DepthBiasState = wgpu::DepthBiasState {
    constant: -2,
    slope_scale: -2.0,
    clamp: 0.0,
};

/// Build a render pipeline from a [`PipelineRecipe`] and the per-pipeline
/// fields (label, layout, shader, vertex buffers, target format).
pub(super) fn pipeline_with_recipe(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    vertex_buffers: &[wgpu::VertexBufferLayout<'_>],
    format: wgpu::TextureFormat,
    recipe: PipelineRecipe,
) -> wgpu::RenderPipeline {
    pipelines::create_render_pipeline(
        device,
        &PipelineDesc {
            label,
            layout,
            shader,
            vertex_buffers,
            format,
            blend: recipe.blend,
            cull_mode: recipe.cull_mode,
            depth: recipe.depth,
        },
    )
}

/// Build a depth-bias overlay pipeline (border, heatmap, water) using
/// [`OVERLAY_DEPTH_BIAS`] together with a custom slope-scale.
pub(super) fn biased_overlay_pipeline(
    device: &wgpu::Device,
    label: &str,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    vertex_buffers: &[wgpu::VertexBufferLayout<'_>],
    format: wgpu::TextureFormat,
    bias: wgpu::DepthBiasState,
) -> wgpu::RenderPipeline {
    pipelines::create_render_pipeline(
        device,
        &PipelineDesc {
            label,
            layout,
            shader,
            vertex_buffers,
            format,
            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
            cull_mode: None,
            depth: DepthConfig::ReadOnlyBiased(bias),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shader_stages_match(a: wgpu::ShaderStages, b: wgpu::ShaderStages) -> bool {
        a == b
    }

    fn entries_match(a: &wgpu::BindGroupLayoutEntry, b: &wgpu::BindGroupLayoutEntry) -> bool {
        a.binding == b.binding
            && shader_stages_match(a.visibility, b.visibility)
            && a.count == b.count
            && format!("{:?}", a.ty) == format!("{:?}", b.ty)
    }

    #[test]
    fn builder_produces_entries_matching_handrolled_descriptor() {
        let built = BindGroupLayoutBuilder::new("test_layout")
            .uniform_buffer(0, wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT)
            .texture_2d_filterable(1, wgpu::ShaderStages::FRAGMENT)
            .sampler_filtering(2, wgpu::ShaderStages::FRAGMENT)
            .sampler_filtering(3, wgpu::ShaderStages::FRAGMENT)
            .entries;

        let hand_rolled = [
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    multisampled: false,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ];
        let hand_rolled = hand_rolled.as_slice();

        assert_eq!(built.len(), hand_rolled.len());
        for (b, h) in built.iter().zip(hand_rolled.iter()) {
            assert!(
                entries_match(b, h),
                "entry mismatch:\nbuilt={b:?}\nhand={h:?}"
            );
        }
    }

    #[test]
    fn builder_depth_helpers() {
        let built = BindGroupLayoutBuilder::new("test_depth")
            .depth_texture(0, wgpu::ShaderStages::FRAGMENT)
            .sampler_comparison(1, wgpu::ShaderStages::FRAGMENT)
            .entries;

        assert_eq!(built.len(), 2);
        assert!(matches!(
            built[0].ty,
            wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            }
        ));
        assert!(matches!(
            built[1].ty,
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison)
        ));
    }

    #[test]
    fn pipeline_recipes_distinct() {
        assert!(matches!(
            PipelineRecipe::TERRAIN_OPAQUE.depth,
            DepthConfig::WriteDefault
        ));
        assert!(matches!(PipelineRecipe::SKY.depth, DepthConfig::ReadOnly));
        assert!(matches!(
            PipelineRecipe::MODEL.cull_mode,
            Some(wgpu::Face::Back)
        ));
    }
}
