use candle_core::Tensor;

pub struct TransformerLayer {
    pub(crate) query_weight: Tensor,
    pub(crate) key_weight: Tensor,
    pub(crate) value_weight: Tensor,
    pub(crate) output_weight: Tensor,
    
    pub(crate) attn_norm_weight: Tensor,
    pub(crate) attn_norm_bias: Option<Tensor>,
    
    pub(crate) ff_inter_weight: Tensor,
    pub(crate) ff_inter_bias: Option<Tensor>,
    
    pub(crate) ff_gate_weight: Option<Tensor>,
    pub(crate) ff_gate_bias: Option<Tensor>,
    
    pub(crate) ff_output_weight: Tensor,
    pub(crate) ff_output_bias: Option<Tensor>,
    
    pub(crate) ff_norm_weight: Tensor,
    pub(crate) ff_norm_bias: Option<Tensor>,
}

impl TransformerLayer {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        query_weight: Tensor,
        key_weight: Tensor,
        value_weight: Tensor,
        output_weight: Tensor,
        attn_norm_weight: Tensor,
        attn_norm_bias: Option<Tensor>,
        ff_inter_weight: Tensor,
        ff_inter_bias: Option<Tensor>,
        ff_gate_weight: Option<Tensor>,
        ff_gate_bias: Option<Tensor>,
        ff_output_weight: Tensor,
        ff_output_bias: Option<Tensor>,
        ff_norm_weight: Tensor,
        ff_norm_bias: Option<Tensor>,
    ) -> Self {
        Self {
            query_weight,
            key_weight,
            value_weight,
            output_weight,
            attn_norm_weight,
            attn_norm_bias,
            ff_inter_weight,
            ff_inter_bias,
            ff_gate_weight,
            ff_gate_bias,
            ff_output_weight,
            ff_output_bias,
            ff_norm_weight,
            ff_norm_bias,
        }
    }
}