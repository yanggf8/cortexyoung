#!/usr/bin/env python3
"""
Cortex V2.1 Embedding Service
FastAPI service for generating semantic embeddings using LangChain
"""

from fastapi import FastAPI, HTTPException
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel
from typing import List, Optional
import numpy as np
import logging
from contextlib import asynccontextmanager

# Configure logging
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

# Global embeddings model
embeddings_model = None

@asynccontextmanager
async def lifespan(app: FastAPI):
    # Startup
    global embeddings_model
    logger.info("🚀 Starting Cortex V2.1 Embedding Service")
    
    try:
        from langchain.embeddings import HuggingFaceEmbeddings
        
        # Initialize with a good code-aware model
        embeddings_model = HuggingFaceEmbeddings(
            model_name="all-MiniLM-L6-v2",  # Fast and decent for code
            model_kwargs={'device': 'cpu'},
            encode_kwargs={'normalize_embeddings': True}
        )
        
        # Test the model
        test_embedding = embeddings_model.embed_query("function test() { return 42; }")
        logger.info(f"✅ Model loaded successfully. Embedding dimension: {len(test_embedding)}")
        
    except Exception as e:
        logger.error(f"❌ Failed to load embedding model: {e}")
        raise e
        
    yield
    
    # Shutdown
    logger.info("🛑 Shutting down Embedding Service")

app = FastAPI(
    title="Cortex V2.1 Embedding Service",
    description="Semantic embedding service for code intelligence",
    version="2.1.0",
    lifespan=lifespan
)

# Add CORS middleware
app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

class EmbedRequest(BaseModel):
    text: str
    
class EmbedBatchRequest(BaseModel):
    texts: List[str]
    
class EmbedResponse(BaseModel):
    embedding: List[float]
    dimension: int
    
class EmbedBatchResponse(BaseModel):
    embeddings: List[List[float]]
    dimension: int
    count: int

@app.get("/health")
async def health_check():
    """Health check endpoint"""
    return {
        "status": "healthy",
        "service": "cortex-embedding-service",
        "version": "2.1.0",
        "model_loaded": embeddings_model is not None
    }

@app.post("/embed", response_model=EmbedResponse)
async def embed_text(request: EmbedRequest):
    """Generate embedding for a single text"""
    if embeddings_model is None:
        raise HTTPException(status_code=503, detail="Embedding model not loaded")
    
    try:
        logger.debug(f"Embedding text of length {len(request.text)}")
        embedding = embeddings_model.embed_query(request.text)
        
        return EmbedResponse(
            embedding=embedding,
            dimension=len(embedding)
        )
    except Exception as e:
        logger.error(f"Failed to generate embedding: {e}")
        raise HTTPException(status_code=500, detail=f"Embedding generation failed: {str(e)}")

@app.post("/embed/batch", response_model=EmbedBatchResponse)
async def embed_batch(request: EmbedBatchRequest):
    """Generate embeddings for multiple texts in batch"""
    if embeddings_model is None:
        raise HTTPException(status_code=503, detail="Embedding model not loaded")
    
    if len(request.texts) > 100:
        raise HTTPException(status_code=400, detail="Batch size too large (max 100)")
    
    try:
        logger.debug(f"Embedding batch of {len(request.texts)} texts")
        embeddings = embeddings_model.embed_documents(request.texts)
        
        return EmbedBatchResponse(
            embeddings=embeddings,
            dimension=len(embeddings[0]) if embeddings else 0,
            count=len(embeddings)
        )
    except Exception as e:
        logger.error(f"Failed to generate batch embeddings: {e}")
        raise HTTPException(status_code=500, detail=f"Batch embedding generation failed: {str(e)}")

@app.get("/models/info")
async def model_info():
    """Get information about the loaded model"""
    if embeddings_model is None:
        raise HTTPException(status_code=503, detail="Embedding model not loaded")
    
    # Test embedding to get dimension
    test_embedding = embeddings_model.embed_query("test")
    
    return {
        "model_name": "all-MiniLM-L6-v2",
        "provider": "HuggingFace",
        "embedding_dimension": len(test_embedding),
        "max_sequence_length": 512,
        "suitable_for": ["code", "text", "semantic_search"]
    }

if __name__ == "__main__":
    import uvicorn
    uvicorn.run(
        "embedding_service:app",
        host="127.0.0.1",
        port=8000,
        reload=True,
        log_level="info"
    )