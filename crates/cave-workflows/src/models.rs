use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Workflow {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
    pub created_at: DateTime<Utc>,
    pub status: WorkflowStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowNode {
    pub id: String,
    pub name: String,
    pub node_type: NodeType,
    pub config: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowEdge {
    pub from_node: String,
    pub to_node: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    Trigger,
    Action,
    Condition,
    Loop,
    End,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowStatus {
    Draft,
    Active,
    Paused,
    Archived,
}

#[derive(Debug, Error)]
pub enum WorkflowError {
    #[error("Cycle detected in workflow DAG")]
    CycleDetected,
    #[error("Node '{0}' referenced in edge but not defined")]
    UndefinedNode(String),
    #[error("Workflow has no nodes")]
    EmptyWorkflow,
    #[error("Multiple trigger nodes found")]
    MultipleTriggers,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_node(id: &str, node_type: NodeType) -> WorkflowNode {
        WorkflowNode {
            id: id.to_string(),
            name: id.to_string(),
            node_type,
            config: HashMap::new(),
        }
    }

    fn make_workflow(nodes: Vec<WorkflowNode>, edges: Vec<WorkflowEdge>) -> Workflow {
        Workflow {
            id: Uuid::new_v4(),
            name: "test".to_string(),
            description: "".to_string(),
            nodes,
            edges,
            created_at: Utc::now(),
            status: WorkflowStatus::Draft,
        }
    }

    #[test]
    fn test_workflow_node_serde_roundtrip() {
        let node = make_node("a", NodeType::Action);
        let json = serde_json::to_string(&node).unwrap();
        let back: WorkflowNode = serde_json::from_str(&json).unwrap();
        assert_eq!(node, back);
    }

    #[test]
    fn test_workflow_edge_serde_roundtrip() {
        let edge = WorkflowEdge { from_node: "a".to_string(), to_node: "b".to_string() };
        let json = serde_json::to_string(&edge).unwrap();
        let back: WorkflowEdge = serde_json::from_str(&json).unwrap();
        assert_eq!(edge, back);
    }

    #[test]
    fn test_workflow_serde_roundtrip() {
        let wf = make_workflow(
            vec![make_node("t", NodeType::Trigger), make_node("a", NodeType::Action)],
            vec![WorkflowEdge { from_node: "t".to_string(), to_node: "a".to_string() }],
        );
        let json = serde_json::to_string(&wf).unwrap();
        let back: Workflow = serde_json::from_str(&json).unwrap();
        assert_eq!(wf, back);
    }

    #[test]
    fn test_node_type_serialization() {
        assert_eq!(serde_json::to_string(&NodeType::Trigger).unwrap(), "\"trigger\"");
        assert_eq!(serde_json::to_string(&NodeType::Action).unwrap(), "\"action\"");
        assert_eq!(serde_json::to_string(&NodeType::Condition).unwrap(), "\"condition\"");
    }

    #[test]
    fn test_workflow_status_serialization() {
        assert_eq!(serde_json::to_string(&WorkflowStatus::Draft).unwrap(), "\"draft\"");
        assert_eq!(serde_json::to_string(&WorkflowStatus::Active).unwrap(), "\"active\"");
        assert_eq!(serde_json::to_string(&WorkflowStatus::Archived).unwrap(), "\"archived\"");
    }

    #[test]
    fn test_node_type_deserialization() {
        let nt: NodeType = serde_json::from_str("\"loop\"").unwrap();
        assert_eq!(nt, NodeType::Loop);
        let nt2: NodeType = serde_json::from_str("\"end\"").unwrap();
        assert_eq!(nt2, NodeType::End);
    }

    #[test]
    fn test_workflow_status_deserialization() {
        let s: WorkflowStatus = serde_json::from_str("\"paused\"").unwrap();
        assert_eq!(s, WorkflowStatus::Paused);
    }

    #[test]
    fn test_workflow_node_with_config() {
        let mut config = HashMap::new();
        config.insert("key".to_string(), serde_json::json!("value"));
        let node = WorkflowNode {
            id: "n1".to_string(),
            name: "Node 1".to_string(),
            node_type: NodeType::Action,
            config,
        };
        let json = serde_json::to_string(&node).unwrap();
        let back: WorkflowNode = serde_json::from_str(&json).unwrap();
        assert_eq!(node, back);
    }

    #[test]
    fn test_workflow_error_display() {
        assert_eq!(WorkflowError::CycleDetected.to_string(), "Cycle detected in workflow DAG");
        assert_eq!(WorkflowError::EmptyWorkflow.to_string(), "Workflow has no nodes");
        assert_eq!(WorkflowError::MultipleTriggers.to_string(), "Multiple trigger nodes found");
        assert_eq!(
            WorkflowError::UndefinedNode("xyz".to_string()).to_string(),
            "Node 'xyz' referenced in edge but not defined"
        );
    }
}
