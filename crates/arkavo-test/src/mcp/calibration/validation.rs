use super::*;
use std::collections::HashMap;

pub struct CalibrationValidator {
    tolerance: ValidationTolerance,
}

#[derive(Debug, Clone)]
pub struct ValidationTolerance {
    pub coordinate_tolerance_pixels: f64,
    pub timing_tolerance_ms: u64,
    pub success_rate_threshold: f64,
}

impl Default for ValidationTolerance {
    fn default() -> Self {
        Self {
            coordinate_tolerance_pixels: 5.0,
            timing_tolerance_ms: 100,
            success_rate_threshold: 0.95,
        }
    }
}

impl Default for CalibrationValidator {
    fn default() -> Self {
        Self::new()
    }
}

impl CalibrationValidator {
    pub fn new() -> Self {
        Self {
            tolerance: ValidationTolerance::default(),
        }
    }
    
    pub fn with_tolerance(mut self, tolerance: ValidationTolerance) -> Self {
        self.tolerance = tolerance;
        self
    }
    
    pub fn validate_calibration(
        &self,
        ground_truth: &GroundTruth,
        interaction_results: &[InteractionTestResult],
    ) -> ValidationReport {
        let mut successful = 0;
        let mut failed = 0;
        let mut issues = Vec::new();
        
        for test_result in interaction_results {
            match self.validate_interaction(ground_truth, test_result) {
                ValidationOutcome::Success => successful += 1,
                ValidationOutcome::Failure(issue) => {
                    failed += 1;
                    issues.push(issue);
                }
            }
        }
        
        let total = successful + failed;
        let accuracy = if total > 0 {
            (successful as f64 / total as f64) * 100.0
        } else {
            0.0
        };
        
        ValidationReport {
            total_interactions: total,
            successful_interactions: successful,
            failed_interactions: failed,
            accuracy_percentage: accuracy,
            issues,
        }
    }
    
    pub fn validate_interaction(
        &self,
        ground_truth: &GroundTruth,
        test_result: &InteractionTestResult,
    ) -> ValidationOutcome {
        // Check if element exists in ground truth
        let expected = match ground_truth.interaction_expectations.get(&test_result.element_id) {
            Some(exp) => exp,
            None => return ValidationOutcome::Failure(ValidationIssue {
                element_id: test_result.element_id.clone(),
                expected_result: "Element in ground truth".to_string(),
                actual_result: "Element not found".to_string(),
                severity: IssueSeverity::Critical,
            }),
        };
        
        // Validate interaction success
        if !test_result.interaction_result.success {
            return ValidationOutcome::Failure(ValidationIssue {
                element_id: test_result.element_id.clone(),
                expected_result: "Successful interaction".to_string(),
                actual_result: "Interaction failed".to_string(),
                severity: IssueSeverity::Major,
            });
        }
        
        // Validate coordinates if available
        if let Some((actual_x, actual_y)) = test_result.interaction_result.actual_coordinates {
            if let Some(expected_coords) = test_result.expected_coordinates {
                let distance = ((actual_x - expected_coords.0).powi(2) + 
                               (actual_y - expected_coords.1).powi(2)).sqrt();
                
                if distance > self.tolerance.coordinate_tolerance_pixels {
                    return ValidationOutcome::Failure(ValidationIssue {
                        element_id: test_result.element_id.clone(),
                        expected_result: format!("Coordinates within {} pixels", 
                                               self.tolerance.coordinate_tolerance_pixels),
                        actual_result: format!("Distance: {:.2} pixels", distance),
                        severity: IssueSeverity::Minor,
                    });
                }
            }
        }
        
        // Validate response time
        if test_result.interaction_result.response_time_ms > self.tolerance.timing_tolerance_ms {
            return ValidationOutcome::Failure(ValidationIssue {
                element_id: test_result.element_id.clone(),
                expected_result: format!("Response time < {}ms", self.tolerance.timing_tolerance_ms),
                actual_result: format!("{}ms", test_result.interaction_result.response_time_ms),
                severity: IssueSeverity::Minor,
            });
        }
        
        // Validate state change if expected
        match &expected.expected_state_change {
            StateChange::None => {
                // No state change expected
            }
            StateChange::ValueChange { from: _, to } => {
                if !test_result.post_interaction_state.contains(to) {
                    return ValidationOutcome::Failure(ValidationIssue {
                        element_id: test_result.element_id.clone(),
                        expected_result: format!("Value changed to '{}'", to),
                        actual_result: "Value unchanged or different".to_string(),
                        severity: IssueSeverity::Major,
                    });
                }
            }
            _ => {
                // Other state changes would be validated here
            }
        }
        
        ValidationOutcome::Success
    }
    
    pub fn compare_calibrations(
        &self,
        old: &CalibrationResult,
        new: &CalibrationResult,
    ) -> CalibrationComparison {
        let accuracy_change = new.validation_report.accuracy_percentage - 
                            old.validation_report.accuracy_percentage;
        
        let mut changed_adjustments = HashMap::new();
        
        // Compare interaction adjustments
        for (element_type, new_adjustment) in &new.interaction_adjustments {
            if let Some(old_adjustment) = old.interaction_adjustments.get(element_type) {
                if !self.adjustments_equal(old_adjustment, new_adjustment) {
                    changed_adjustments.insert(
                        element_type.clone(),
                        AdjustmentChange {
                            old: old_adjustment.clone(),
                            new: new_adjustment.clone(),
                        },
                    );
                }
            } else {
                // New adjustment added
                changed_adjustments.insert(
                    element_type.clone(),
                    AdjustmentChange {
                        old: InteractionAdjustment {
                            element_type: element_type.clone(),
                            tap_offset: None,
                            requires_double_tap: false,
                            requires_long_press: false,
                            custom_delay_ms: None,
                        },
                        new: new_adjustment.clone(),
                    },
                );
            }
        }
        
        let adjustment_count = changed_adjustments.len();
        let needs_recalibration = accuracy_change < -5.0 || // 5% accuracy drop
                                 adjustment_count > 3; // Many changes
        
        CalibrationComparison {
            old_version: old.device_profile.os_version.clone(),
            new_version: new.device_profile.os_version.clone(),
            accuracy_change,
            changed_adjustments,
            new_edge_cases: new.edge_cases.len() - old.edge_cases.len(),
            recommendation: if needs_recalibration {
                CalibrationRecommendation::FullRecalibration
            } else if adjustment_count == 0 {
                CalibrationRecommendation::NoChangesNeeded
            } else {
                CalibrationRecommendation::MinorAdjustments
            },
        }
    }
    
    fn adjustments_equal(&self, a: &InteractionAdjustment, b: &InteractionAdjustment) -> bool {
        a.tap_offset == b.tap_offset &&
        a.requires_double_tap == b.requires_double_tap &&
        a.requires_long_press == b.requires_long_press &&
        a.custom_delay_ms == b.custom_delay_ms
    }
}

#[derive(Debug, Clone)]
pub struct InteractionTestResult {
    pub element_id: String,
    pub interaction_result: InteractionResult,
    pub expected_coordinates: Option<(f64, f64)>,
    pub post_interaction_state: String,
}

#[derive(Debug)]
pub enum ValidationOutcome {
    Success,
    Failure(ValidationIssue),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationComparison {
    pub old_version: String,
    pub new_version: String,
    pub accuracy_change: f64,
    pub changed_adjustments: HashMap<String, AdjustmentChange>,
    pub new_edge_cases: usize,
    pub recommendation: CalibrationRecommendation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdjustmentChange {
    pub old: InteractionAdjustment,
    pub new: InteractionAdjustment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CalibrationRecommendation {
    NoChangesNeeded,
    MinorAdjustments,
    FullRecalibration,
}

pub struct ValidationHelpers;

impl ValidationHelpers {
    pub fn generate_validation_matrix(
        elements: &[UIElement],
    ) -> Vec<ValidationTestCase> {
        let mut test_cases = Vec::new();
        
        for element in elements {
            // Basic tap test for all elements
            test_cases.push(ValidationTestCase {
                name: format!("Tap {}", element.id),
                element_id: element.id.clone(),
                action: CalibrationAction {
                    action_type: ActionType::Tap,
                    target: ActionTarget::ElementId(element.id.clone()),
                    parameters: HashMap::new(),
                },
                expected_outcome: ExpectedOutcome::ElementTapped,
            });
            
            // Additional tests based on element type
            match &element.element_type {
                ElementType::Switch | ElementType::Checkbox => {
                    // Test toggle behavior
                    test_cases.push(ValidationTestCase {
                        name: format!("Toggle {}", element.id),
                        element_id: element.id.clone(),
                        action: CalibrationAction {
                            action_type: ActionType::Tap,
                            target: ActionTarget::ElementId(element.id.clone()),
                            parameters: HashMap::new(),
                        },
                        expected_outcome: ExpectedOutcome::StateToggled,
                    });
                }
                ElementType::TextField => {
                    // Test text input
                    test_cases.push(ValidationTestCase {
                        name: format!("Type in {}", element.id),
                        element_id: element.id.clone(),
                        action: CalibrationAction {
                            action_type: ActionType::TypeText,
                            target: ActionTarget::ElementId(element.id.clone()),
                            parameters: {
                                let mut params = HashMap::new();
                                params.insert("text".to_string(), serde_json::json!("test"));
                                params
                            },
                        },
                        expected_outcome: ExpectedOutcome::TextEntered,
                    });
                }
                _ => {}
            }
        }
        
        test_cases
    }
}

#[derive(Debug, Clone)]
pub struct ValidationTestCase {
    pub name: String,
    pub element_id: String,
    pub action: CalibrationAction,
    pub expected_outcome: ExpectedOutcome,
}

#[derive(Debug, Clone)]
pub enum ExpectedOutcome {
    ElementTapped,
    StateToggled,
    TextEntered,
    NavigationOccurred,
    Custom(String),
}